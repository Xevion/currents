use crate::settings::CameraTuning;
use crate::view::View;

const MIN_HALF_HEIGHT: f32 = 0.002;
/// Pixel scroll distance that counts as one line.
pub(crate) const PIXELS_PER_LINE: f32 = 40.0;
/// Share of the view's height within which the shown view counts as arrived, well under a pixel.
const ARRIVED: f32 = 2e-5;
/// How far past the region input may ask for, in regions; only keeps the numbers finite.
const REACH: f32 = 4.0;

/// How far a push of `over` past an edge shows: one for one at first, levelling off at `limit`.
/// The push meets more resistance the further it goes and never gets beyond the limit.
fn give(over: f32, limit: f32) -> f32 {
    if limit <= 0.0 { 0.0 } else { limit * over / (over + limit) }
}

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

    /// This framing as it shows while pushed past the region: what lies inside is unchanged,
    /// and what lies past an edge gives a little and then holds.
    fn yielding(mut self, viewport: Viewport, limit: f32) -> Self {
        let region = viewport.region();
        let over = self.half_height / region[1] - 1.0;
        if over > 0.0 {
            self.half_height = region[1] * (1.0 + give(over, limit));
        }
        self.half_height = self.half_height.max(MIN_HALF_HEIGHT);
        let half = self.half(viewport);
        for axis in 0..2 {
            let slack = (region[axis] - half[axis]).max(0.0);
            let offset = self.centre[axis] - 0.5;
            let size = 2.0 * half[axis];
            let past = (offset.abs() - slack) / size;
            if past > 0.0 {
                self.centre[axis] = 0.5 + offset.signum() * size.mul_add(give(past, limit), slack);
            }
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
///
/// The target may leave the region views stay in. The view drawn follows it a little way past
/// the edge, and the target drifts back inside whenever nothing holds it there.
#[derive(Clone, Copy)]
pub(crate) struct Camera {
    target: Framing,
    shown: Framing,
    /// Whether a drag is in progress, which keeps the target where it was put.
    held: bool,
    pub(crate) tuning: CameraTuning,
}

impl Default for Camera {
    fn default() -> Self {
        Self { target: Framing::WHOLE, shown: Framing::WHOLE, held: false, tuning: CameraTuning::default() }
    }
}

impl Camera {
    pub(crate) fn new(tuning: CameraTuning) -> Self {
        Self { tuning, ..Self::default() }
    }

    /// The view to draw.
    pub(crate) fn view(&self, viewport: Viewport) -> View {
        View::from_centre(self.shown.centre, self.shown.half(viewport))
    }

    /// The cover point drawn under `cursor`, in window pixels.
    pub(crate) fn cover_point(&self, cursor: [f64; 2], viewport: Viewport) -> [f32; 2] {
        let [x0, y0, x1, y1] = self.view(viewport).to_array();
        let (u, v) = (cursor[0] as f32 / viewport.width, cursor[1] as f32 / viewport.height);
        [(x1 - x0).mul_add(u, x0), (y1 - y0).mul_add(v, y0)]
    }

    pub(crate) fn reset(&mut self) {
        self.target = Framing::WHOLE;
    }

    /// Marks whether a drag holds the view; once let go, a view past the edge comes back.
    pub(crate) fn hold(&mut self, held: bool) {
        self.held = held;
    }

    /// Moves the drawn view toward the target over `dt` seconds, after letting a target that
    /// nothing holds past the edge drift back inside.
    pub(crate) fn advance(&mut self, dt: f32, viewport: Viewport) {
        if !self.held {
            let inside = self.target.bounded(viewport);
            let next = self.target.toward(&inside, 1.0 - (-dt / self.tuning.return_secs).exp());
            self.target = if next.arrived_at(&inside) { inside } else { next };
        }
        let goal = self.target.yielding(viewport, self.tuning.give);
        let next = self.shown.toward(&goal, 1.0 - (-dt / self.tuning.ease_secs).exp());
        self.shown = if next.arrived_at(&goal) { goal } else { next };
    }

    /// Drags the picture along with a cursor that moved by `delta` pixels.
    pub(crate) fn pan(&mut self, delta: [f64; 2], viewport: Viewport) {
        let half = self.target.half(viewport);
        let region = viewport.region();
        let moved =
            [delta[0] as f32 / viewport.width * 2.0 * half[0], delta[1] as f32 / viewport.height * 2.0 * half[1]];
        for axis in 0..2 {
            let reach = REACH * region[axis];
            self.target.centre[axis] = (self.target.centre[axis] - moved[axis]).clamp(0.5 - reach, 0.5 + reach);
        }
    }

    /// Zooms in by `lines` scroll lines (out when negative), keeping the point under `cursor` fixed.
    pub(crate) fn zoom(&mut self, lines: f32, cursor: [f64; 2], viewport: Viewport) {
        self.scale_extent(self.tuning.zoom_per_line.powf(-lines), cursor, viewport);
    }

    /// Multiplies the visible extent by `factor`, keeping the point under `cursor` fixed.
    pub(crate) fn scale_extent(&mut self, factor: f32, cursor: [f64; 2], viewport: Viewport) {
        let ndc = [cursor[0] as f32 / viewport.width * 2.0 - 1.0, cursor[1] as f32 / viewport.height * 2.0 - 1.0];
        let before = self.target.half(viewport);
        let anchor = [self.target.centre[0] + ndc[0] * before[0], self.target.centre[1] + ndc[1] * before[1]];
        let widest = REACH * viewport.region()[1];
        self.target.half_height = (self.target.half_height * factor).clamp(MIN_HALF_HEIGHT, widest);
        let after = self.target.half(viewport);
        self.target.centre = [anchor[0] - ndc[0] * after[0], anchor[1] - ndc[1] * after[1]];
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
        Camera { target: framing, shown: framing, ..Camera::default() }
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
    fn zoom_comes_to_rest_at_the_limits(#[case] lines: f32, #[case] limit: f32) {
        let viewport = Viewport::new(900, 900);
        let mut camera = Camera::default();
        camera.zoom(lines, [100.0, 200.0], viewport);
        for _ in 0..600 {
            camera.advance(1.0 / 60.0, viewport);
        }
        check!(camera.shown.half_height.total_cmp(&limit).is_eq());
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

    /// Whether `view` lies within the region grown by `give` of its own size on each side.
    fn within(view: [f32; 4], viewport: Viewport, give: f32) -> bool {
        let region = viewport.region();
        (0..2).all(|axis| {
            let allowed = give.mul_add(view[axis + 2] - view[axis], region[axis]) + TOLERANCE;
            view[axis] >= 0.5 - allowed && view[axis + 2] <= 0.5 + allowed
        })
    }

    #[rstest]
    fn a_view_pushed_past_the_region_gives_a_little_and_comes_back_when_let_go(
        #[values((900, 900), (1600, 600), (500, 1000))] size: (u32, u32),
        #[values([1.0, 0.0], [-1.0, 0.0], [0.0, 1.0], [0.0, -1.0], [1.0, 1.0])] toward: [f64; 2],
        #[values(-100.0, -3.0, 4.0)] lines: f32,
    ) {
        let viewport = Viewport::new(size.0, size.1);
        let mut camera = Camera::default();
        camera.hold(true);
        camera.zoom(lines, [0.0, 0.0], viewport);
        camera.pan(toward.map(|v| v * 1e6), viewport);
        for _ in 0..200 {
            camera.advance(1.0 / 60.0, viewport);
            // a zoom past the limit and a pan past the edge each give at most the tuned limit
            check!(within(camera.view(viewport).to_array(), viewport, 2.0 * camera.tuning.give));
        }
        check!(!within(camera.view(viewport).to_array(), viewport, 0.0));
        camera.hold(false);
        for _ in 0..600 {
            camera.advance(1.0 / 60.0, viewport);
        }
        check!(within(camera.view(viewport).to_array(), viewport, 0.0));
        // the cover fits the region exactly along the window's short side
        let region = viewport.region();
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
