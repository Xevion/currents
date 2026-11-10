use crate::view::View;

const MIN_HALF_HEIGHT: f32 = 0.002;
/// Zoom factor applied per scroll line.
const ZOOM_PER_LINE: f32 = 1.15;
/// Pixel scroll distance that counts as one line.
pub(crate) const PIXELS_PER_LINE: f32 = 40.0;
/// Seconds in which the shown view closes about two thirds of the way to where input put it.
const EASE_SECS: f32 = 0.08;
/// Share of the view's height within which the shown view counts as arrived, well under a pixel.
const ARRIVED: f32 = 2e-5;

/// Window size in pixels, as floats.
#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    width: f32,
    height: f32,
}

impl Viewport {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        Self { width: width as f32, height: height as f32 }
    }

    fn aspect(self) -> f32 {
        self.width / self.height
    }

    /// Half the width and height of the region views stay in: the smallest rectangle of the
    /// window's shape that holds the whole cover, centred on it.
    fn region(self) -> [f32; 2] {
        [0.5 * self.aspect().max(1.0), 0.5 * self.aspect().recip().max(1.0)]
    }
}

/// A rectangle of the cover: a centre and a half height, with the width following the window.
#[derive(Clone, Copy, PartialEq)]
struct Framing {
    centre: [f32; 2],
    half_height: f32,
}

impl Framing {
    const WHOLE: Self = Self { centre: [0.5, 0.5], half_height: 0.5 };

    fn half(&self, viewport: Viewport) -> [f32; 2] {
        [self.half_height * viewport.aspect(), self.half_height]
    }

    /// This framing pulled inside the region views stay in.
    fn bounded(mut self, viewport: Viewport) -> Self {
        let region = viewport.region();
        self.half_height = self.half_height.clamp(MIN_HALF_HEIGHT, region[1]);
        let half = self.half(viewport);
        for axis in 0..2 {
            let slack = (region[axis] - half[axis]).max(0.0);
            self.centre[axis] = self.centre[axis].clamp(0.5 - slack, 0.5 + slack);
        }
        self
    }

    /// The framing `share` of the way from this one to `other`. Centre and extent move by the
    /// same share, so the point a zoom holds fixed stays fixed all the way there.
    fn toward(&self, other: &Self, share: f32) -> Self {
        let mix = |a: f32, b: f32| (b - a).mul_add(share, a);
        Self {
            centre: [mix(self.centre[0], other.centre[0]), mix(self.centre[1], other.centre[1])],
            half_height: mix(self.half_height, other.half_height),
        }
    }

    fn arrived_at(&self, other: &Self) -> bool {
        let far = (self.centre[0] - other.centre[0])
            .abs()
            .max((self.centre[1] - other.centre[1]).abs())
            .max((self.half_height - other.half_height).abs());
        far <= ARRIVED * other.half_height
    }
}

/// The visible rectangle of the cover. Input moves `target` at once; the view drawn eases
/// after it, so a scroll notch or a jerk of the pointer reads as one smooth move.
#[derive(Clone, Copy)]
pub(crate) struct Camera {
    target: Framing,
    shown: Framing,
}

impl Default for Camera {
    fn default() -> Self {
        Self { target: Framing::WHOLE, shown: Framing::WHOLE }
    }
}

impl Camera {
    /// The view to draw.
    pub(crate) fn view(&self, viewport: Viewport) -> View {
        View::from_centre(self.shown.centre, self.shown.half(viewport))
    }

    pub(crate) fn reset(&mut self) {
        self.target = Framing::WHOLE;
    }

    /// Moves the drawn view toward the target over `dt` seconds, and keeps the target inside
    /// the region for a window that may have changed shape.
    pub(crate) fn advance(&mut self, dt: f32, viewport: Viewport) {
        self.target = self.target.bounded(viewport);
        let next = self.shown.toward(&self.target, 1.0 - (-dt / EASE_SECS).exp()).bounded(viewport);
        self.shown = if next.arrived_at(&self.target) { self.target } else { next };
    }

    /// Drags the picture along with a cursor that moved by `delta` pixels.
    pub(crate) fn pan(&mut self, delta: [f64; 2], viewport: Viewport) {
        let half = self.target.half(viewport);
        self.target.centre[0] -= delta[0] as f32 / viewport.width * 2.0 * half[0];
        self.target.centre[1] -= delta[1] as f32 / viewport.height * 2.0 * half[1];
        self.target = self.target.bounded(viewport);
    }

    /// Zooms in by `lines` scroll lines (out when negative), keeping the point under `cursor` fixed.
    pub(crate) fn zoom(&mut self, lines: f32, cursor: [f64; 2], viewport: Viewport) {
        self.scale_extent(ZOOM_PER_LINE.powf(-lines), cursor, viewport);
    }

    /// Multiplies the visible extent by `factor`, keeping the point under `cursor` fixed
    /// unless that would take the view out of its region.
    pub(crate) fn scale_extent(&mut self, factor: f32, cursor: [f64; 2], viewport: Viewport) {
        let ndc = [cursor[0] as f32 / viewport.width * 2.0 - 1.0, cursor[1] as f32 / viewport.height * 2.0 - 1.0];
        let before = self.target.half(viewport);
        let anchor = [self.target.centre[0] + ndc[0] * before[0], self.target.centre[1] + ndc[1] * before[1]];
        self.target.half_height = (self.target.half_height * factor).clamp(MIN_HALF_HEIGHT, viewport.region()[1]);
        let after = self.target.half(viewport);
        self.target.centre = [anchor[0] - ndc[0] * after[0], anchor[1] - ndc[1] * after[1]];
        self.target = self.target.bounded(viewport);
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    /// f32 carries about 6e-8 relative error and these sums stay below 2 in magnitude, so a few
    /// roundings stay under 1e-6; a wrong anchor moves the point by 1e-2 or more.
    const TOLERANCE: f32 = 1e-5;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < TOLERANCE
    }

    fn point_under(framing: &Framing, viewport: Viewport, cursor: [f64; 2]) -> [f32; 2] {
        let half = framing.half(viewport);
        let ndc =
            [cursor[0] / f64::from(viewport.width) * 2.0 - 1.0, cursor[1] / f64::from(viewport.height) * 2.0 - 1.0];
        [framing.centre[0] + ndc[0] as f32 * half[0], framing.centre[1] + ndc[1] as f32 * half[1]]
    }

    fn at(centre: [f32; 2], half_height: f32) -> Camera {
        let framing = Framing { centre, half_height };
        Camera { target: framing, shown: framing }
    }

    #[rstest]
    fn zoom_keeps_the_point_under_the_cursor_fixed_all_the_way_there(
        #[values((900, 900), (1600, 600), (500, 1000))] size: (u32, u32),
        #[values((0.0, 0.0), (0.5, 0.5), (1.0, 0.25), (0.8, 0.9))] cursor: (f64, f64),
        #[values(-2.0, 1.0, 5.0)] lines: f32,
    ) {
        let viewport = Viewport::new(size.0, size.1);
        let cursor = [cursor.0 * f64::from(size.0), cursor.1 * f64::from(size.1)];
        let mut camera = at([0.5, 0.55], 0.1);
        let before = point_under(&camera.target, viewport, cursor);
        camera.zoom(lines, cursor, viewport);
        let after = point_under(&camera.target, viewport, cursor);
        check!(close(before[0], after[0]) && close(before[1], after[1]));
        check!((camera.target.half_height - 0.1).abs() > 1e-3);
        for _ in 0..200 {
            camera.advance(1.0 / 60.0, viewport);
            let during = point_under(&camera.shown, viewport, cursor);
            check!(close(before[0], during[0]) && close(before[1], during[1]));
        }
        check!(camera.shown == camera.target);
    }

    #[rstest]
    #[case::zoom_in(100.0, MIN_HALF_HEIGHT)]
    #[case::zoom_out(-100.0, 0.5)]
    fn zoom_stops_at_the_limits(#[case] lines: f32, #[case] limit: f32) {
        let mut camera = Camera::default();
        camera.zoom(lines, [100.0, 200.0], Viewport::new(900, 900));
        check!(camera.target.half_height.total_cmp(&limit).is_eq());
    }

    #[rstest]
    #[case(900, 900)]
    #[case(1600, 600)]
    #[case(500, 1000)]
    fn panning_moves_the_centre_by_the_share_of_the_view_dragged(#[case] width: u32, #[case] height: u32) {
        let viewport = Viewport::new(width, height);
        let mut camera = at([0.5, 0.5], 0.1);
        let visible = camera.target.half(viewport).map(|h| 2.0 * h);
        camera.pan([f64::from(width) / 4.0, f64::from(height) / 4.0], viewport);
        check!(close(camera.target.centre[0], 0.5 - visible[0] / 4.0));
        check!(close(camera.target.centre[1], 0.5 - visible[1] / 4.0));
    }

    #[rstest]
    fn the_view_never_leaves_the_smallest_window_shaped_region_holding_the_cover(
        #[values((900, 900), (1600, 600), (500, 1000))] size: (u32, u32),
        #[values([1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0], [1.0, 1.0])] toward: [f64; 2],
        #[values(-100.0, -3.0, 4.0)] lines: f32,
    ) {
        let viewport = Viewport::new(size.0, size.1);
        let region = viewport.region();
        let mut camera = Camera::default();
        camera.zoom(lines, [0.0, 0.0], viewport);
        camera.pan(toward.map(|v| v * 1e6), viewport);
        for _ in 0..200 {
            camera.advance(1.0 / 60.0, viewport);
            let view = camera.view(viewport).to_array();
            check!(view[0] >= 0.5 - region[0] - TOLERANCE && view[2] <= 0.5 + region[0] + TOLERANCE);
            check!(view[1] >= 0.5 - region[1] - TOLERANCE && view[3] <= 0.5 + region[1] + TOLERANCE);
        }
        // the cover fits the region exactly along the window's short side
        check!(close(region[0].min(region[1]), 0.5));
    }

    #[test]
    fn reset_eases_back_to_the_default_view() {
        let viewport = Viewport::new(1280, 720);
        let mut camera = Camera::default();
        camera.zoom(4.0, [300.0, 100.0], viewport);
        camera.pan([50.0, -20.0], viewport);
        camera.advance(1.0, viewport);
        camera.reset();
        camera.advance(1.0 / 60.0, viewport);
        check!(camera.view(viewport) != Camera::default().view(viewport));
        for _ in 0..200 {
            camera.advance(1.0 / 60.0, viewport);
        }
        check!(camera.view(viewport) == Camera::default().view(viewport));
    }
}
