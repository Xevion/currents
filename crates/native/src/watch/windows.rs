//! The surroundings on Windows: the foreground window over the viewer, as a game or a video
//! fills the monitor, the viewer minimised, and battery power.

use currents_core::viewer::Conditions;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::UI::Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetShellWindow, GetWindowRect, IsIconic,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::{Failing, Probe, Rect, over};

/// `ACLineStatus` when the machine runs on battery; 1 is mains and 255 unknown.
const AC_OFFLINE: u8 = 0;
/// Classes of the windows that draw the desktop itself, which never count as covering it.
const DESKTOP_CLASSES: [&str; 2] = ["Progman", "WorkerW"];

pub(crate) struct Win32Probe {
    /// The viewer's window, kept as a number so the probe can move to its own thread.
    ours: isize,
    embedded: bool,
    failing: [Failing; 3],
}

const fn hwnd(id: isize) -> HWND {
    id as HWND
}

fn rect_of(window: HWND) -> Result<Rect, String> {
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: `r` is a valid RECT to write; an invalid handle makes the call fail, not crash.
    if unsafe { GetWindowRect(window, &raw mut r) } == 0 {
        return Err("GetWindowRect failed".to_owned());
    }
    Ok((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn monitor_of(window: HWND) -> Result<Rect, String> {
    // SAFETY: plain handle arguments; the nearest monitor is always returned.
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    // SAFETY: MONITORINFO is plain data that the call fills once `cbSize` is set.
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = u32::try_from(size_of::<MONITORINFO>()).unwrap_or(0);
    // SAFETY: `info` is a valid MONITORINFO with its size set.
    if unsafe { GetMonitorInfoW(monitor, &raw mut info) } == 0 {
        return Err("GetMonitorInfoW failed".to_owned());
    }
    let r = info.rcMonitor;
    Ok((r.left, r.top, r.right - r.left, r.bottom - r.top))
}

fn class_of(window: HWND) -> String {
    let mut name = [0u16; 64];
    // SAFETY: writes at most the buffer's length of UTF-16 units.
    let len = unsafe { GetClassNameW(window, name.as_mut_ptr(), 64) };
    String::from_utf16_lossy(&name[..usize::try_from(len).unwrap_or(0)])
}

impl Win32Probe {
    pub(crate) fn new(window: &Window, embedded: bool) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else { return None };
        Some(Self { ours: handle.hwnd.get(), embedded, failing: Default::default() })
    }

    fn fullscreen(&self) -> Result<bool, String> {
        let mut state = 0;
        // SAFETY: writes one enum value into `state`.
        let exclusive =
            unsafe { SHQueryUserNotificationState(&raw mut state) } == 0 && state == QUNS_RUNNING_D3D_FULL_SCREEN;
        // SAFETY: no arguments; returns null when no window is in the foreground.
        let front = unsafe { GetForegroundWindow() };
        // SAFETY: no arguments.
        let shell = unsafe { GetShellWindow() };
        if front.is_null()
            || front == hwnd(self.ours)
            || front == shell
            || DESKTOP_CLASSES.contains(&class_of(front).as_str())
        {
            return Ok(false);
        }
        // A program in exclusive fullscreen covers its monitor, whatever its window says.
        let theirs = if exclusive { monitor_of(front)? } else { rect_of(front)? };
        Ok(over(theirs, rect_of(hwnd(self.ours))?, exclusive))
    }

    fn hidden(&self) -> bool {
        // SAFETY: a plain handle argument.
        !self.embedded && unsafe { IsIconic(hwnd(self.ours)) } != 0
    }
}

fn on_battery() -> Result<bool, String> {
    // SAFETY: SYSTEM_POWER_STATUS is plain data the call fills.
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: `status` is a valid SYSTEM_POWER_STATUS.
    if unsafe { GetSystemPowerStatus(&raw mut status) } == 0 {
        return Err("GetSystemPowerStatus failed".to_owned());
    }
    Ok(status.ACLineStatus == AC_OFFLINE)
}

impl Probe for Win32Probe {
    fn sample(&mut self) -> Conditions {
        let fullscreen = self.fullscreen();
        let hidden = self.hidden();
        let [f, h, b] = &mut self.failing;
        Conditions {
            fullscreen: f.check("fullscreen window", fullscreen),
            hidden: h.check::<String>("minimised", Ok(hidden)),
            on_battery: b.check("battery", on_battery()),
        }
    }
}
