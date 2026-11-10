use web_time::Instant;

use crate::renderer::PassTimes;

/// Seconds between window title updates.
const TITLE_INTERVAL: f32 = 0.5;

/// Counts frames and produces the window title text once per interval.
pub(crate) struct FrameCounter {
    frames: u32,
    since: Instant,
}

impl FrameCounter {
    pub(crate) fn new(now: Instant) -> Self {
        Self { frames: 0, since: now }
    }

    /// Counts one frame drawn at `now`; returns the title when the interval has elapsed.
    pub(crate) fn tick(&mut self, now: Instant, passes: PassTimes, time: f32, paused: bool) -> Option<String> {
        self.frames += 1;
        let dt = (now - self.since).as_secs_f32();
        if dt <= TITLE_INTERVAL {
            return None;
        }
        let state = if paused { "paused" } else { "running" };
        let PassTimes { field: f, shade: s } = passes;
        let title = format!(
            "currents | {:.0} fps | field {f:.1}ms shade {s:.1}ms | t={time:.1} | {state}",
            self.frames as f32 / dt,
        );
        self.frames = 0;
        self.since = now;
        Some(title)
    }
}
