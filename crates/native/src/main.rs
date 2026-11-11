// A release build on Windows opens no console of its own, since as a wallpaper it would sit
// beside the desktop all day; run from a terminal, it writes to that terminal instead.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[snafu::report]
fn main() -> Result<(), currents::Error> {
    currents::run()
}
