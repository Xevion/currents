//! When the viewer draws: at the frame rate the settings allow, slower on battery, or not at
//! all while nothing would see the frames or the host holds it still.

use crate::settings::Settings;

/// What the platform reports about the viewer's surroundings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Conditions {
    /// The window is minimised, or covered where the window toolkit cannot tell.
    pub hidden: bool,
    /// Another program fills the monitor the viewer is on.
    pub fullscreen: bool,
    pub on_battery: bool,
}

/// How the next frame is paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pacing {
    /// Draw, held to a frame rate when there is one.
    Run(Option<u32>),
    /// Draw nothing until something changes.
    Idle,
}

/// The pacing `settings` ask for given the surroundings; `occluded` is what the window
/// toolkit reports, `held` whether the host asked the viewer to stop, and `attended` whether
/// the menu or a message is up, which keeps a window that is still in view drawing.
pub(crate) fn pacing(
    settings: &Settings,
    conditions: Conditions,
    occluded: bool,
    held: bool,
    attended: bool,
) -> Pacing {
    let w = &settings.wallpaper;
    let unseen = (occluded || conditions.hidden) && w.pause_when_hidden;
    let covered = conditions.fullscreen && w.pause_under_fullscreen;
    let unplugged = conditions.on_battery && w.pause_on_battery;
    if held || unseen || ((covered || unplugged) && !attended) {
        return Pacing::Idle;
    }
    let cap = settings.render.fps_cap;
    let battery = w.battery_fps_cap.filter(|_| conditions.on_battery);
    Pacing::Run(match (cap, battery) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    })
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    fn settings(fps: Option<u32>, battery_fps: Option<u32>, pause_on_battery: bool) -> Settings {
        let mut s = Settings::default();
        s.render.fps_cap = fps;
        s.wallpaper.battery_fps_cap = battery_fps;
        s.wallpaper.pause_on_battery = pause_on_battery;
        s
    }

    const PLUGGED: Conditions = Conditions { hidden: false, fullscreen: false, on_battery: false };
    const BATTERY: Conditions = Conditions { on_battery: true, ..PLUGGED };

    #[rstest]
    #[case::free(settings(None, None, false), PLUGGED, Pacing::Run(None))]
    #[case::limited(settings(Some(60), None, false), PLUGGED, Pacing::Run(Some(60)))]
    #[case::battery_limit_waits_for_battery(settings(None, Some(30), false), PLUGGED, Pacing::Run(None))]
    #[case::battery_limit(settings(None, Some(30), false), BATTERY, Pacing::Run(Some(30)))]
    #[case::the_lower_limit_wins(settings(Some(24), Some(30), false), BATTERY, Pacing::Run(Some(24)))]
    #[case::battery_lower(settings(Some(60), Some(30), false), BATTERY, Pacing::Run(Some(30)))]
    #[case::battery_without_a_limit(settings(Some(60), None, false), BATTERY, Pacing::Run(Some(60)))]
    #[case::paused_on_battery(settings(None, Some(30), true), BATTERY, Pacing::Idle)]
    #[case::the_battery_pause_waits_for_the_menu_to_close(settings(None, None, true), BATTERY, Pacing::Idle)]
    #[case::pause_on_battery_while_plugged(settings(None, None, true), PLUGGED, Pacing::Run(None))]
    fn frames_follow_the_limits_and_the_power(
        #[case] settings: Settings,
        #[case] conditions: Conditions,
        #[case] expected: Pacing,
    ) {
        check!(pacing(&settings, conditions, false, false, false) == expected);
    }

    #[rstest]
    fn each_reason_to_stop_stops_only_while_its_setting_is_on(
        #[values(false, true)] hidden: bool,
        #[values(false, true)] occluded: bool,
        #[values(false, true)] fullscreen: bool,
        #[values(false, true)] held: bool,
        #[values(false, true)] pause_when_hidden: bool,
        #[values(false, true)] pause_under_fullscreen: bool,
        #[values(false, true)] attended: bool,
    ) {
        let mut s = Settings::default();
        s.wallpaper.pause_when_hidden = pause_when_hidden;
        s.wallpaper.pause_under_fullscreen = pause_under_fullscreen;
        let conditions = Conditions { hidden, fullscreen, on_battery: false };
        // The menu keeps a window that is still in view drawing, but cannot show through a
        // window nobody can see.
        let stops =
            held || ((hidden || occluded) && pause_when_hidden) || (fullscreen && pause_under_fullscreen && !attended);
        let expected = if stops { Pacing::Idle } else { Pacing::Run(None) };
        check!(pacing(&s, conditions, occluded, held, attended) == expected);
    }
}
