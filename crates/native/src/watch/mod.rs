//! Watches what the window toolkit cannot report, for the viewer's pause settings: a program
//! filling the monitor, the window being minimised, and the computer running on battery.

#[cfg(target_os = "linux")]
mod battery;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

use std::time::Duration;

use currents_core::viewer::{Conditions, Remote};
use winit::window::Window;

/// A rectangle on the screen: x, y, width, height.
pub(crate) type Rect = (i32, i32, i32, i32);

/// Whether a window at `theirs` keeps the viewer at `ours` from being seen: it covers the
/// viewer whole, or is marked fullscreen and overlaps it at all, as on the viewer's monitor.
pub(crate) fn over(theirs: Rect, ours: Rect, marked_fullscreen: bool) -> bool {
    let (tx, ty, tw, th) = theirs;
    let (ox, oy, ow, oh) = ours;
    let covers = tx <= ox && ty <= oy && tx + tw >= ox + ow && ty + th >= oy + oh;
    let overlaps = tx < ox + ow && ox < tx + tw && ty < oy + oh && oy < ty + th;
    covers || (marked_fullscreen && overlaps)
}

/// How often the surroundings are looked at; a pause a second late costs nothing.
const EVERY: Duration = Duration::from_secs(1);

/// One look at the surroundings. A check that fails reads as false, so a broken check never
/// pauses the viewer.
pub(crate) trait Probe {
    fn sample(&mut self) -> Conditions;
}

/// Logs a check's failure the first time it happens, and the recovery after.
#[derive(Default)]
pub(crate) struct Failing(bool);

impl Failing {
    pub(crate) fn check<E: std::fmt::Display>(&mut self, what: &str, result: Result<bool, E>) -> bool {
        match result {
            Ok(value) => {
                if std::mem::take(&mut self.0) {
                    tracing::info!(check = what, "check works again");
                }
                value
            },
            Err(error) => {
                if !std::mem::replace(&mut self.0, true) {
                    tracing::warn!(check = what, %error, "check failed; it reads as false until it works");
                }
                false
            },
        }
    }
}

/// Starts watching on a thread of its own, telling the viewer each time something changes.
/// `embedded` skips the minimised check, which a window inside another cannot be.
pub(crate) fn spawn(window: &Window, remote: Remote, embedded: bool) {
    let Some(mut probe) = probe(window, embedded) else {
        tracing::warn!("nothing to watch the surroundings with on this platform");
        return;
    };
    std::thread::Builder::new()
        .name("watch".to_owned())
        .spawn(move || {
            let mut last = None;
            loop {
                let now = probe.sample();
                if last != Some(now) {
                    if last.is_none() {
                        tracing::info!(conditions = ?now, "watching the surroundings");
                    }
                    remote.conditions(now);
                    last = Some(now);
                }
                std::thread::sleep(EVERY);
            }
        })
        .map_err(|error| tracing::warn!(%error, "could not start watching the surroundings"))
        .ok();
}

#[cfg(target_os = "linux")]
fn probe(window: &Window, embedded: bool) -> Option<Box<dyn Probe + Send>> {
    let id = crate::x11::xid(window)?;
    match linux::X11Probe::new(id, embedded) {
        Ok(probe) => Some(Box::new(probe)),
        Err(error) => {
            tracing::warn!(%error, "cannot watch the X11 desktop");
            None
        },
    }
}

#[cfg(windows)]
fn probe(window: &Window, embedded: bool) -> Option<Box<dyn Probe + Send>> {
    windows::Win32Probe::new(window, embedded).map(|p| Box::new(p) as Box<dyn Probe + Send>)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn probe(_: &Window, _: bool) -> Option<Box<dyn Probe + Send>> {
    None
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    const MONITOR: Rect = (0, 0, 1920, 1080);

    #[rstest]
    #[case::a_fullscreen_game(MONITOR, MONITOR, false, true)]
    #[case::fullscreen_on_another_monitor((1920, 0, 1920, 1080), MONITOR, true, false)]
    #[case::marked_fullscreen_here((0, 0, 1280, 720), MONITOR, true, true)]
    #[case::a_maximised_window_below_a_panel((0, 32, 1920, 1048), MONITOR, false, false)]
    #[case::covering_a_small_viewer_window((0, 0, 1920, 1080), (100, 100, 800, 600), false, true)]
    #[case::beside_a_small_viewer_window((1000, 0, 900, 1080), (100, 100, 800, 600), false, false)]
    fn a_window_keeps_the_viewer_unseen_only_when_over_it(
        #[case] theirs: Rect,
        #[case] ours: Rect,
        #[case] marked: bool,
        #[case] expected: bool,
    ) {
        check!(over(theirs, ours, marked) == expected);
    }
}
