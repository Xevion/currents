use winit::dpi::PhysicalSize;

/// Seconds of frames averaged before each decision.
const WINDOW_SECS: f32 = 0.5;
/// Frame intervals longer than this are stalls (a hidden tab, a shader compile), not load.
const STALL_SECS: f32 = 0.25;
/// Mean intervals beyond this mean the GPU is not keeping up.
const SLOW_SECS: f32 = 1.0 / 45.0;
/// Mean intervals under this leave headroom to render more pixels.
const FAST_SECS: f32 = 1.0 / 57.0;
/// Consecutive fast windows needed before scaling back up.
const FAST_WINDOWS: u32 = 4;
const DOWN: f64 = 0.8;
const UP: f64 = 1.1;
const MIN_SCALE: f64 = 0.3;
/// How close under the last scale that was too slow the scale may climb back.
const CEILING_MARGIN: f64 = 0.95;
/// Output pixels per field texel once frames have run long. The field pass is the larger
/// part of a frame and this quarters it, softening only the thinnest filaments of the stream.
const COARSE_FIELD_STEP: u32 = 2;

/// Caps the work per frame: a fixed pixel budget, then cheaper frames while they run long.
///
/// Slow frames first coarsen the flow field, which stays coarse from then on, and after that
/// shrink a per-axis scale that recovers once frames have headroom again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget {
    pub max_pixels: f64,
    /// Starting per-axis scale, at most 1.
    pub scale: f64,
    pub adaptive: bool,
}

pub(crate) struct RenderScale {
    budget: Option<Budget>,
    scale: f64,
    ceiling: f64,
    frames: u32,
    elapsed: f32,
    fast: u32,
    field_step: u32,
}

impl RenderScale {
    pub(crate) fn new(budget: Option<Budget>, field_step: u32) -> Self {
        let scale = budget.map_or(1.0, |b| b.scale.clamp(MIN_SCALE, 1.0));
        Self { budget, scale, ceiling: 1.0, frames: 0, elapsed: 0.0, fast: 0, field_step: field_step.max(1) }
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

    /// Counts a frame that took `dt` seconds; true when the scale or the field step changed.
    pub(crate) fn observe(&mut self, dt: f32) -> bool {
        if !self.budget.is_some_and(|b| b.adaptive) || dt > STALL_SECS {
            return false;
        }
        self.frames += 1;
        self.elapsed += dt;
        if self.elapsed < WINDOW_SECS {
            return false;
        }
        let mean = self.elapsed / self.frames as f32;
        (self.frames, self.elapsed) = (0, 0.0);
        let before = self.scale;
        if mean > SLOW_SECS && self.field_step < COARSE_FIELD_STEP {
            self.field_step = COARSE_FIELD_STEP;
            self.fast = 0;
            return true;
        }
        if mean > SLOW_SECS {
            self.ceiling = self.scale;
            self.scale = (self.scale * DOWN).max(MIN_SCALE);
            self.fast = 0;
        } else if mean < FAST_SECS {
            self.fast += 1;
            if self.fast >= FAST_WINDOWS {
                self.fast = 0;
                self.scale = (self.scale * UP).min(self.ceiling * CEILING_MARGIN).max(self.scale);
            }
        } else {
            self.fast = 0;
        }
        self.scale.total_cmp(&before).is_ne()
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    const ADAPTIVE: Budget = Budget { max_pixels: 1e6, scale: 1.0, adaptive: true };

    fn run(scale: &mut RenderScale, dt: f32, secs: f32) -> bool {
        let mut changed = false;
        for _ in 0..(secs / dt) as u32 {
            changed |= scale.observe(dt);
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
    fn the_field_stays_coarse_after_frames_recover() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 30.0, 0.6);
        run(&mut scale, 1.0 / 120.0, 30.0);
        check!(scale.field_step() == COARSE_FIELD_STEP);
    }

    #[test]
    fn recovery_stops_short_of_the_scale_that_was_too_slow() {
        let mut scale = RenderScale::new(Some(ADAPTIVE), 1);
        run(&mut scale, 1.0 / 30.0, 1.2);
        let ceiling = 1.0;
        run(&mut scale, 1.0 / 120.0, 30.0);
        check!(scale.scale() < ceiling);
        check!(scale.scale() > DOWN);
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
