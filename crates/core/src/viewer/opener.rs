//! Opening the menu without a keyboard: hold the right button on the ball, or a finger anywhere.

use web_time::{Duration, Instant};

/// The ball's centre and radius in cover coordinates; `common.wgsl` draws it from the same numbers.
pub(crate) const BALL_C: [f32; 2] = [0.543, 0.661];
pub(crate) const BALL_R: f32 = 0.1205;

/// How long the button has to stay down on the ball.
const HOLD: Duration = Duration::from_secs(1);
/// How long a finger has to stay down, about the platform's long press.
pub(crate) const TOUCH_HOLD: Duration = Duration::from_millis(500);

/// Whether the cover point `p` lies on the ball.
pub(crate) fn on_ball(p: [f32; 2]) -> bool {
    let (dx, dy) = (p[0] - BALL_C[0], p[1] - BALL_C[1]);
    dx.hypot(dy) <= BALL_R
}

/// A hold in progress: when it started and how long it has to last.
#[derive(Default)]
pub(crate) struct Opener {
    since: Option<(Instant, Duration)>,
}

impl Opener {
    /// The button went down; only a press on the ball starts a hold.
    pub(crate) fn press(&mut self, now: Instant, on_ball: bool) {
        self.since = on_ball.then_some((now, HOLD));
    }

    /// A finger went down; it opens the menu after a shorter hold, wherever it is.
    pub(crate) fn touch(&mut self, now: Instant) {
        self.since = Some((now, TOUCH_HOLD));
    }

    /// The pointer moved; leaving the ball drops the hold.
    pub(crate) fn moved(&mut self, on_ball: bool) {
        if !on_ball {
            self.since = None;
        }
    }

    pub(crate) fn release(&mut self) {
        self.since = None;
    }

    /// The share of the hold done, while one is in progress.
    pub(crate) fn progress(&self, now: Instant) -> Option<f32> {
        self.since.map(|(t, hold)| ((now - t).as_secs_f32() / hold.as_secs_f32()).min(1.0))
    }

    /// True once when a hold completes, which also ends it.
    pub(crate) fn fired(&mut self, now: Instant) -> bool {
        let done = self.since.is_some_and(|(t, hold)| now - t >= hold);
        if done {
            self.since = None;
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn the_ball_constants_match_the_shader() {
        let common = include_str!("../../../../shaders/common.wgsl");
        check!(common.contains(&format!("const BALL_C = vec2f({}, {});", BALL_C[0], BALL_C[1])));
        check!(common.contains(&format!("const BALL_R = {BALL_R};")));
    }

    #[test]
    fn a_full_hold_on_the_ball_fires_once() {
        let t = Instant::now();
        let mut opener = Opener::default();
        opener.press(t, true);
        check!(!opener.fired(t + HOLD / 2));
        check!(opener.progress(t + HOLD / 2).is_some_and(|p| (p - 0.5).abs() < 1e-3));
        check!(opener.fired(t + HOLD));
        check!(!opener.fired(t + HOLD * 2));
        check!(opener.progress(t + HOLD * 2).is_none());
    }

    #[test]
    fn releasing_early_pressing_elsewhere_or_sliding_off_cancels() {
        let t = Instant::now();
        let mut early = Opener::default();
        early.press(t, true);
        early.release();
        check!(!early.fired(t + HOLD));

        let mut elsewhere = Opener::default();
        elsewhere.press(t, false);
        check!(!elsewhere.fired(t + HOLD));

        let mut slid = Opener::default();
        slid.press(t, true);
        slid.moved(false);
        check!(!slid.fired(t + HOLD));
    }

    #[test]
    fn a_touch_hold_fires_sooner_than_the_button_hold() {
        let t = Instant::now();
        let mut opener = Opener::default();
        opener.touch(t);
        check!(!opener.fired(t + TOUCH_HOLD / 2));
        check!(opener.fired(t + TOUCH_HOLD));
    }

    #[test]
    fn the_ball_is_a_circle_round_its_centre() {
        check!(on_ball(BALL_C));
        check!(on_ball([BALL_C[0] + 0.9 * BALL_R, BALL_C[1]]));
        check!(!on_ball([BALL_C[0] + 0.8 * BALL_R, BALL_C[1] + 0.8 * BALL_R]));
    }
}
