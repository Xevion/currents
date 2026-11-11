//! What the window toolkit leaves undone on X11: the hints that keep a desktop window on every
//! workspace and out of taskbars and pagers, and following the size of a window the viewer is
//! embedded in.

use std::error::Error;

use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask, PropMode, Window as Xid,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// `_NET_WM_DESKTOP` for a window shown on every workspace.
const ALL_DESKTOPS: u32 = 0xFFFF_FFFF;
/// `_NET_WM_STATE` asks to add the atoms it carries.
const STATE_ADD: u32 = 1;
/// A client message from an ordinary application, as the window manager spec asks.
const SOURCE_APPLICATION: u32 = 1;

/// The X11 window behind `window`, when it is one.
pub(crate) fn xid(window: &Window) -> Option<Xid> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Xid::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

fn atom(conn: &RustConnection, name: &str) -> Result<Atom> {
    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
}

/// The desktop window's states: kept below the rest, on every workspace and in no taskbar or pager.
const DESKTOP_STATES: [&str; 4] =
    ["_NET_WM_STATE_BELOW", "_NET_WM_STATE_STICKY", "_NET_WM_STATE_SKIP_TASKBAR", "_NET_WM_STATE_SKIP_PAGER"];

/// Marks `window` as the desktop's: below every other, on every workspace, and in no taskbar
/// or pager. A window manager is asked, and the properties are also written for one that reads
/// them only as a window maps.
pub(crate) fn desktop_hints(conn: &RustConnection, root: Xid, window: Xid) -> Result<()> {
    let wm_state = atom(conn, "_NET_WM_STATE")?;
    let wm_desktop = atom(conn, "_NET_WM_DESKTOP")?;
    let states = DESKTOP_STATES.iter().map(|name| atom(conn, name)).collect::<Result<Vec<_>>>()?;
    let ask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    for pair in states.chunks(2) {
        let data = [STATE_ADD, pair[0], pair.get(1).copied().unwrap_or(0), SOURCE_APPLICATION, 0];
        conn.send_event(false, root, ask, ClientMessageEvent::new(32, window, wm_state, data))?;
    }
    let data = [ALL_DESKTOPS, SOURCE_APPLICATION, 0, 0, 0];
    conn.send_event(false, root, ask, ClientMessageEvent::new(32, window, wm_desktop, data))?;

    let mut held: Vec<Atom> = conn
        .get_property(false, window, wm_state, AtomEnum::ATOM, 0, 64)?
        .reply()?
        .value32()
        .map(Iterator::collect)
        .unwrap_or_default();
    for state in states {
        if !held.contains(&state) {
            held.push(state);
        }
    }
    conn.change_property32(PropMode::REPLACE, window, wm_state, AtomEnum::ATOM, &held)?;
    conn.change_property32(PropMode::REPLACE, window, wm_desktop, AtomEnum::CARDINAL, &[ALL_DESKTOPS])?;
    conn.flush()?;
    Ok(())
}

/// Marks `window` as the desktop's over a connection of its own.
pub(crate) fn mark_desktop(window: Xid) -> Result<()> {
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;
    desktop_hints(&conn, root, window)
}

/// Follows `parent`'s size, as `follow` does, over a connection of its own.
pub(crate) fn follow_parent(parent: Xid, resize: impl Fn(u32, u32)) -> Result<()> {
    let (conn, _) = x11rb::connect(None)?;
    follow(&conn, parent, resize)
}

/// Calls `resize` with `parent`'s size now and on every change, until the parent goes away.
/// Blocks, so it runs on a thread of its own.
pub(crate) fn follow(conn: &RustConnection, parent: Xid, resize: impl Fn(u32, u32)) -> Result<()> {
    conn.change_window_attributes(parent, &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY))?;
    let geometry = conn.get_geometry(parent)?.reply()?;
    resize(u32::from(geometry.width), u32::from(geometry.height));
    conn.flush()?;
    let mut last = (geometry.width, geometry.height);
    loop {
        match conn.wait_for_event()? {
            Event::ConfigureNotify(e) if e.window == parent && (e.width, e.height) != last => {
                last = (e.width, e.height);
                resize(u32::from(e.width), u32::from(e.height));
            },
            Event::DestroyNotify(e) if e.window == parent => return Ok(()),
            _ => {},
        }
    }
}

/// A window ID as xwinwrap and others pass one: decimal, or hexadecimal after `0x`.
pub(crate) fn parse_xid(text: &str) -> std::result::Result<u32, String> {
    let text = text.trim();
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.ok().filter(|&id| id != 0).ok_or_else(|| format!("{text} is not an X11 window ID"))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use assert2::{assert, check};
    use rstest::rstest;
    use x11rb::COPY_DEPTH_FROM_PARENT;
    use x11rb::protocol::xproto::{ConfigureWindowAux, CreateWindowAux, WindowClass};

    use super::*;

    #[rstest]
    #[case("12345", Some(12345))]
    #[case("0x3a00007", Some(0x3a0_0007))]
    #[case(" 0X1F ", Some(0x1f))]
    #[case("0", None)]
    #[case("WID", None)]
    #[case("0x", None)]
    fn window_ids_read_as_xwinwrap_writes_them(#[case] text: &str, #[case] id: Option<u32>) {
        check!(parse_xid(text).ok() == id);
    }

    /// A connection to the virtual display `just test-display` gives, never the desktop's.
    fn display() -> (RustConnection, Xid) {
        assert!(
            std::env::var_os("CURRENTS_TEST_DISPLAY").is_some(),
            "opens windows; run it with `just test-display`, which gives it a virtual display"
        );
        let (conn, screen) = x11rb::connect(None).expect("an X display");
        let root = conn.setup().roots[screen].root;
        (conn, root)
    }

    fn window(conn: &RustConnection, root: Xid, size: u16) -> Xid {
        let id = conn.generate_id().unwrap();
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            id,
            root,
            0,
            0,
            size,
            size,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap();
        conn.map_window(id).unwrap();
        conn.flush().unwrap();
        id
    }

    #[test]
    #[ignore = "opens windows; run with `just test-display`"]
    fn a_desktop_window_holds_every_state_a_wallpaper_needs() {
        let (conn, root) = display();
        let win = window(&conn, root, 64);
        desktop_hints(&conn, root, win).unwrap();
        let wm_state = atom(&conn, "_NET_WM_STATE").unwrap();
        let held: Vec<Atom> = conn
            .get_property(false, win, wm_state, AtomEnum::ATOM, 0, 64)
            .unwrap()
            .reply()
            .unwrap()
            .value32()
            .unwrap()
            .collect();
        for name in DESKTOP_STATES {
            check!(held.contains(&atom(&conn, name).unwrap()), "{name}");
        }
        let wm_desktop = atom(&conn, "_NET_WM_DESKTOP").unwrap();
        let desktop = conn.get_property(false, win, wm_desktop, AtomEnum::CARDINAL, 0, 1).unwrap().reply().unwrap();
        check!(desktop.value32().unwrap().next() == Some(ALL_DESKTOPS));
    }

    #[test]
    #[ignore = "opens windows; run with `just test-display`"]
    fn an_embedded_viewer_follows_its_parent_until_it_goes() {
        let (conn, root) = display();
        let parent = window(&conn, root, 300);
        let (sizes, seen) = mpsc::channel();
        let watcher = std::thread::spawn(move || {
            let (conn, _) = x11rb::connect(None).unwrap();
            follow(&conn, parent, |w, h| sizes.send((w, h)).unwrap())
        });
        let next = || seen.recv_timeout(Duration::from_secs(5)).expect("a size");
        check!(next() == (300, 300));
        conn.configure_window(parent, &ConfigureWindowAux::new().width(640).height(360)).unwrap();
        conn.flush().unwrap();
        check!(next() == (640, 360));
        conn.destroy_window(parent).unwrap();
        conn.flush().unwrap();
        assert!(let Ok(Ok(())) = watcher.join());
    }
}
