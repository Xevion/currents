use winit::dpi::PhysicalSize;

/// Seconds of frames averaged before each decision.
const WINDOW_SECS: f32 = 0.5;
/// Frame intervals longer than this are stalls (a hidden tab, a shader compile), not load.
const STALL_SECS: f32 = 0.25;
/// Frame intervals beyond this mean frames are not keeping up.
const SLOW_INTERVAL: f32 = 1.0 / 45.0;
/// GPU time per frame beyond this leaves too little of a 60 Hz frame for anything else.
const SLOW_GPU: f32 = 1.0 / 70.0;
/// Share of the slow mark under which a window counts as having headroom.
const FAST_SHARE: f32 = 0.7;
/// Consecutive fast windows needed before scaling back up.
const FAST_WINDOWS: u32 = 4;
const DOWN: f64 = 0.8;
const UP: f64 = 1.1;
/// The lowest scale adapting goes to; below it the stream's stripes break into moire.
const MIN_SCALE: f64 = 0.5;
/// How close under the last scale that was too slow the scale may climb back.
const CEILING_MARGIN: f64 = 0.95;
/// Seconds without a slow window before the ceiling lifts and the field sharpens again.
const LIFT_SECS: f32 = 5.0;
/// The longest wait before trying again, after lifts that kept turning out too slow.
const MAX_LIFT_SECS: f32 = 60.0;
/// Output pixels per field texel once frames have run long. The field pass is the larger
/// part of a frame and this quarters it, softening only the thinnest filaments of the stream.
const COARSE_FIELD_STEP: u32 = 2;

/// Caps the work per frame: a fixed pixel budget, then cheaper frames while they run long.
///
/// Slow frames first coarsen the flow field and then shrink a per-axis scale. Both recover:
/// the scale climbs while frames have headroom, and after a calm spell the field sharpens and
/// the scale may try the size that was once too slow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub max_pixels: f64,
    /// Starting per-axis scale, at most 1.
    pub scale: f64,
    pub adaptive: bool,
}

pub(crate) struct RenderScale {
    budget: Option<Budget>,
    /// The scale and field step asked for, which recovery climbs back to.
    top: f64,
    fine_step: u32,
    scale: f64,
    ceiling: f64,
    frames: u32,
    /// Sum of each frame's load as a share of the slow mark.
    pressure: f32,
    elapsed: f32,
    fast: u32,
    /// Seconds since the last slow window.
    calm: f32,
    lift_after: f32,
    /// Whether the last change was a lift, so a slow window right after it backs off.
    lifted: bool,
    field_step: u32,
}

impl RenderScale {
    pub(crate) fn new(budget: Option<Budget>, field_step: u32) -> Self {
        let top = budget.map_or(1.0, |b| b.scale.clamp(0.05, 1.0));
        let fine_step = field_step.max(1);
        Self {
            budget,
            top,
            fine_step,
            scale: top,
            ceiling: top,
            frames: 0,
            pressure: 0.0,
            elapsed: 0.0,
            fast: 0,
            calm: 0.0,
            lift_after: LIFT_SECS,
            lifted: false,
            field_step: fine_step,
        }
    }

    /// Output pixels per flow field texel.
    pub(crate) fn field_step(&self) -> u32 {
        self.field_step
    }

    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    /// The size to render a window of `physical` pixels at.
    pub(crate) fn fit(&self, physical: PhysicalSize<u32>) -> PhysicalSize<u32> {
        let (w, h) = (f64::from(physical.width.max(1)), f64::from(physical.height.max(1)));
        let Some(budget) = self.budget else { return PhysicalSize::new(w as u32, h as u32) };
        let k = self.scale * (budget.max_pixels / (w * h)).sqrt().min(1.0);
        let scaled = |v: f64| (v * k).round().max(1.0) as u32;
        PhysicalSize::new(scaled(w), scaled(h))
    }

    /// Drops the frames counted so far, for when they say nothing about the GPU: the page
    /// was hidden or unfocused and the browser slowed its frames.
    pub(crate) fn pause(&mut self) {
        (self.frames, self.pressure, self.elapsed) = (0, 0.0, 0.0);
    }

    /// Counts a frame that came `dt` seconds after the last and spent `gpu` seconds in its
    /// passes, when the device times them; true when the scale or the field step changed.
    /// GPU time is the better measure, since the browser may space frames out on its own.
    pub(crate) fn observe(&mut self, dt: f32, gpu: Option<f32>) -> bool {
        if !self.budget.is_some_and(|b| b.adaptive) || dt > STALL_SECS {
            return false;
        }
        self.frames += 1;
        self.elapsed += dt;
        self.pressure += gpu.filter(|&g| g > 0.0).map_or(dt / SLOW_INTERVAL, |g| g / SLOW_GPU);
        if self.elapsed < WINDOW_SECS {
            return false;
        }
        let mean = self.pressure / self.frames as f32;
        let window = self.elapsed;
        self.pause();
        let before = (self.scale, self.field_step);
        if mean > 1.0 {
            self.slow();
        } else {
            self.calm += window;
            self.fast = if mean < FAST_SHARE { self.fast + 1 } else { 0 };
            if self.fast >= FAST_WINDOWS {
                self.fast = 0;
                let cap = if self.ceiling >= self.top { self.top } else { self.ceiling * CEILING_MARGIN };
                self.scale = (self.scale * UP).min(cap).max(self.scale);
            }
            if self.calm >= self.lift_after {
                self.lift();
            }
        }
        (self.scale, self.field_step) != before
    }

    fn slow(&mut self) {
        self.fast = 0;
        self.calm = 0.0;
        if std::mem::take(&mut self.lifted) {
            self.lift_after = (self.lift_after * 2.0).min(MAX_LIFT_SECS);
        }
        if self.field_step < COARSE_FIELD_STEP {
            self.field_step = COARSE_FIELD_STEP;
            return;
        }
        self.ceiling = self.scale;
        self.scale = (self.scale * DOWN).max(MIN_SCALE.min(self.top));
    }

    /// After a calm spell: let the scale climb past where it was once too slow, or sharpen the
    /// field once the scale is back at the top.
    fn lift(&mut self) {
        self.calm = 0.0;
        if self.ceiling < self.top {
            self.ceiling = self.top;
            self.lifted = true;
        } else if self.scale >= self.top && self.field_step != self.fine_step {
            self.field_step = self.fine_step;
            self.lifted = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    const ADAPTIVE: Budget = Budget { max_pixels: 1e6, scale: 1.0, adaptive: true };

    fn run(scale: &mut RenderScale, dt: f32, secs: f32) -> bool {
        timed(scale, dt, None, secs)
    }

    fn timed(scale: &mut RenderScale, dt: f32, gpu: Option<f32>, secs: f32) -> bool {
        let mut changed = false;
        for _ in 0..(secs / dt) as u32 {
            changed |= scale.observe(dt, gpu);
        }
        changed
    }

    #[test]
    fn native_windows_render_every_pixel() {
        check!(RenderScale::new(None, 1).fit(PhysicalSize::new(3840, 2160)) == PhysicalSize::new(3840, 2160));
    }

    #[test]
    fn the_budget_caps_pixels_and_keeps_the_aspect() {
        let size = RenderScale::new(Some(ADAPTIVE), 1).fit(PhysicalSize::new(2000, 1000));
        check!(f64::from(size.width) * f64::from(size.height) <= 1.001e6);
        check!(size.width == 2 * size.height);
    }

    #[test]
    fn slow_frames_coarsen_the_field_before_shrinking_the_scale() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        check!(!run(&mut scale, 1.0 / 60.0, 3.0));
        check!(run(&mut scale, 1.0 / 30.0, 0.6));
        check!((scale.field_step(), scale.scale() < 1.0) == (COARSE_FIELD_STEP, false));
        check!(run(&mut scale, 1.0 / 30.0, 0.6));
        check!((scale.field_step(), scale.scale() < 1.0) == (COARSE_FIELD_STEP, true));
    }

    #[test]
    fn a_long_slow_spell_never_drops_below_the_floor() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 20.0, 60.0);
        check!(scale.scale().total_cmp(&MIN_SCALE).is_eq());
    }

    #[test]
    fn everything_recovers_once_frames_have_headroom_again() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 20.0, 10.0);
        check!(scale.scale().total_cmp(&MIN_SCALE).is_eq());
        run(&mut scale, 1.0 / 120.0, 30.0);
        check!((scale.scale(), scale.field_step()) == (1.0, 1));
    }

    #[test]
    fn recovery_stops_short_of_the_scale_that_was_too_slow_until_a_calm_spell() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 30.0, 1.2);
        run(&mut scale, 1.0 / 120.0, 0.9 * LIFT_SECS);
        check!(scale.scale() < 1.0);
        check!(scale.scale() > DOWN);
    }

    #[test]
    fn a_lift_that_proves_too_slow_waits_longer_before_the_next() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 30.0, 1.2);
        run(&mut scale, 1.0 / 120.0, 2.0 * LIFT_SECS);
        check!(scale.field_step() == 1);
        run(&mut scale, 1.0 / 30.0, 0.6);
        check!(scale.field_step() == COARSE_FIELD_STEP);
        check!(scale.lift_after.total_cmp(&(2.0 * LIFT_SECS)).is_eq());
    }

    #[test]
    fn frames_the_browser_spaces_out_leave_the_scale_alone_when_the_gpu_is_quick() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        check!(!timed(&mut scale, 1.0 / 20.0, Some(0.004), 10.0));
        check!(timed(&mut scale, 1.0 / 60.0, Some(0.03), 1.2));
    }

    #[test]
    fn stalls_and_fixed_budgets_never_change_the_scale() {
        let mut adaptive = RenderScale::new(Some(ADAPTIVE), 1);
        check!(!run(&mut adaptive, 0.5, 10.0));
        let mut fixed = RenderScale::new(Some(Budget { adaptive: false, ..ADAPTIVE }), 1);
        check!(!run(&mut fixed, 1.0 / 20.0, 5.0));
        check!(fixed.field_step() == 1);
    }
}
