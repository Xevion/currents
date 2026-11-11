//! The surroundings on an X11 desktop: the active window filling the viewer's area, the
//! viewer minimised, and battery power.

use std::path::Path;

use currents_core::viewer::Conditions;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window as Xid};
use x11rb::rust_connection::RustConnection;

use super::{Failing, Probe, Rect, battery, over};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Atoms {
    active: Atom,
    state: Atom,
    fullscreen: Atom,
    hidden: Atom,
    window_type: Atom,
    desktop: Atom,
}

pub(crate) struct X11Probe {
    conn: RustConnection,
    root: Xid,
    ours: Xid,
    atoms: Atoms,
    embedded: bool,
    failing: [Failing; 3],
}

impl X11Probe {
    pub(crate) fn new(ours: Xid, embedded: bool) -> Result<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &str| -> Result<Atom> { Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom) };
        let atoms = Atoms {
            active: atom("_NET_ACTIVE_WINDOW")?,
            state: atom("_NET_WM_STATE")?,
            fullscreen: atom("_NET_WM_STATE_FULLSCREEN")?,
            hidden: atom("_NET_WM_STATE_HIDDEN")?,
            window_type: atom("_NET_WM_WINDOW_TYPE")?,
            desktop: atom("_NET_WM_WINDOW_TYPE_DESKTOP")?,
        };
        Ok(Self { conn, root, ours, atoms, embedded, failing: Default::default() })
    }

    fn atoms_of(&self, window: Xid, property: Atom) -> Result<Vec<Atom>> {
        let reply = self.conn.get_property(false, window, property, AtomEnum::ATOM, 0, 64)?.reply()?;
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    /// Where `window` sits on the root window.
    fn rect(&self, window: Xid) -> Result<Rect> {
        let geometry = self.conn.get_geometry(window)?.reply()?;
        let at = self.conn.translate_coordinates(window, self.root, 0, 0)?.reply()?;
        Ok((i32::from(at.dst_x), i32::from(at.dst_y), i32::from(geometry.width), i32::from(geometry.height)))
    }

    fn fullscreen(&self) -> Result<bool> {
        let reply = self.conn.get_property(false, self.root, self.atoms.active, AtomEnum::WINDOW, 0, 1)?.reply()?;
        let Some(active) = reply.value32().and_then(|mut v| v.next()).filter(|&w| w != 0 && w != self.ours) else {
            return Ok(false);
        };
        if self.atoms_of(active, self.atoms.window_type)?.contains(&self.atoms.desktop) {
            return Ok(false);
        }
        let marked = self.atoms_of(active, self.atoms.state)?.contains(&self.atoms.fullscreen);
        Ok(over(self.rect(active)?, self.rect(self.ours)?, marked))
    }

    fn hidden(&self) -> Result<bool> {
        Ok(!self.embedded && self.atoms_of(self.ours, self.atoms.state)?.contains(&self.atoms.hidden))
    }
}

impl Probe for X11Probe {
    fn sample(&mut self) -> Conditions {
        let fullscreen = self.fullscreen();
        let hidden = self.hidden();
        let [f, h, b] = &mut self.failing;
        Conditions {
            fullscreen: f.check("fullscreen window", fullscreen),
            hidden: h.check("minimised", hidden),
            on_battery: b.check("battery", battery::on_battery(Path::new(battery::SUPPLIES))),
        }
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use x11rb::COPY_DEPTH_FROM_PARENT;
    use x11rb::protocol::xproto::{CreateWindowAux, PropMode, WindowClass};
    use x11rb::wrapper::ConnectionExt as _;

    use super::*;

    /// An unmapped window: the probe reads only properties and geometry, and a mapped one this
    /// size would cover the windows of other tests sharing the display.
    fn window(conn: &RustConnection, root: Xid, width: u16, height: u16) -> Xid {
        let id = conn.generate_id().unwrap();
        let aux = CreateWindowAux::new();
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            id,
            root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &aux,
        )
        .unwrap();
        settle(conn);
        id
    }

    /// Waits until the server has handled everything sent, so the probe's own connection sees it.
    fn settle(conn: &RustConnection) {
        conn.get_input_focus().unwrap().reply().unwrap();
    }

    #[test]
    #[ignore = "opens windows; run with `just test-display`"]
    fn a_fullscreen_active_window_and_a_minimised_viewer_are_seen() {
        assert!(std::env::var_os("CURRENTS_TEST_DISPLAY").is_some(), "run it with `just test-display`");
        let (conn, screen) = x11rb::connect(None).unwrap();
        let root = conn.setup().roots[screen].root;
        let ours = window(&conn, root, 640, 480);
        let mut probe = X11Probe::new(ours, false).unwrap();
        check!(!probe.sample().fullscreen);

        // With no window manager on the test display, the test sets what one would.
        let game = window(&conn, root, 1920, 1080);
        let a = &probe.atoms;
        conn.change_property32(PropMode::REPLACE, game, a.state, AtomEnum::ATOM, &[a.fullscreen]).unwrap();
        conn.change_property32(PropMode::REPLACE, root, a.active, AtomEnum::WINDOW, &[game]).unwrap();
        settle(&conn);
        check!(probe.sample().fullscreen);

        let desktop_icons = window(&conn, root, 1920, 1080);
        let a = &probe.atoms;
        conn.change_property32(PropMode::REPLACE, desktop_icons, a.window_type, AtomEnum::ATOM, &[a.desktop]).unwrap();
        conn.change_property32(PropMode::REPLACE, root, a.active, AtomEnum::WINDOW, &[desktop_icons]).unwrap();
        settle(&conn);
        check!(!probe.sample().fullscreen);

        let a = &probe.atoms;
        conn.change_property32(PropMode::REPLACE, ours, a.state, AtomEnum::ATOM, &[a.hidden]).unwrap();
        settle(&conn);
        check!(probe.sample().hidden);
        probe.embedded = true;
        check!(!probe.sample().hidden);
    }
}
