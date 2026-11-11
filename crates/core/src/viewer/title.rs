use web_time::Instant;

use crate::renderer::{FieldView, PassTimes};

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
    pub(crate) fn tick(
        &mut self,
        now: Instant,
        passes: Option<PassTimes>,
        time: f64,
        paused: bool,
        view: Option<FieldView>,
    ) -> Option<String> {
        self.frames += 1;
        let dt = (now - self.since).as_secs_f32();
        if dt <= TITLE_INTERVAL {
            return None;
        }
        let state = if paused { "paused" } else { "running" };
        let view: &str = view.map_or("shaded", Into::into);
        let passes = passes.map_or_else(
            || "pass times unavailable".to_owned(),
            |PassTimes { field: f, shade: s }| format!("field {f:.1}ms shade {s:.1}ms"),
        );
        let title =
            format!("currents | {:.0} fps | {passes} | t={time:.1} | {state} | {view}", self.frames as f32 / dt);
        self.frames = 0;
        self.since = now;
        Some(title)
    }
}

/// Weight of the newest frame in the frame rate the menu shows.
const SMOOTHING: f32 = 0.05;

/// The frame rate over the last moment. It averages the time between frames and inverts that,
/// since averaging each frame's own rate reads high whenever frames come unevenly.
#[derive(Default)]
pub(crate) struct FrameRate {
    interval: Option<f32>,
}

impl FrameRate {
    pub(crate) fn observe(&mut self, dt: f32) {
        if dt > 0.0 {
            let mean = self.interval.get_or_insert(dt);
            *mean += SMOOTHING * (dt - *mean);
        }
    }

    pub(crate) fn fps(&self) -> f32 {
        self.interval.map_or(0.0, f32::recip)
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn uneven_frames_read_at_the_rate_they_actually_come() {
        let mut rate = FrameRate::default();
        // Thirty frames a second, on average, but alternating quick and slow.
        for i in 0..2000 {
            rate.observe(if i % 2 == 0 { 1.0 / 60.0 } else { 1.0 / 20.0 });
        }
        check!((rate.fps() - 30.0).abs() < 1.0, "{}", rate.fps());
    }
}
