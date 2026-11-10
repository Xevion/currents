use crate::cli::View;

const MIN_HALF_HEIGHT: f32 = 0.002;
const MAX_HALF_HEIGHT: f32 = 4.0;
/// Zoom factor applied per scroll line.
const ZOOM_PER_LINE: f32 = 1.15;
/// Pixel scroll distance that counts as one line.
pub(super) const PIXELS_PER_LINE: f32 = 40.0;

/// Window size in pixels, as floats.
#[derive(Clone, Copy)]
pub(super) struct Viewport {
    width: f32,
    height: f32,
}

impl Viewport {
    pub(super) fn new(width: u32, height: u32) -> Self {
        Self { width: width as f32, height: height as f32 }
    }
}

/// The visible rectangle of the cover: a centre and a half height, with the width following the window.
#[derive(Clone, Copy)]
pub(super) struct Camera {
    centre: [f32; 2],
    half_height: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { centre: [0.5, 0.5], half_height: 0.5 }
    }
}

impl Camera {
    /// Half the visible width and height, in cover coordinates.
    pub(super) fn half(&self, viewport: Viewport) -> [f32; 2] {
        [self.half_height * viewport.width / viewport.height, self.half_height]
    }

    pub(super) fn view(&self, viewport: Viewport) -> View {
        View::from_centre(self.centre, self.half(viewport))
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Drags the picture along with a cursor that moved by `delta` pixels.
    pub(super) fn pan(&mut self, delta: [f64; 2], viewport: Viewport) {
        let half = self.half(viewport);
        self.centre[0] -= delta[0] as f32 / viewport.width * 2.0 * half[0];
        self.centre[1] -= delta[1] as f32 / viewport.height * 2.0 * half[1];
    }

    /// Zooms in by `lines` scroll lines (out when negative), keeping the point under `cursor` fixed.
    pub(super) fn zoom(&mut self, lines: f32, cursor: [f64; 2], viewport: Viewport) {
        let ndc = [cursor[0] as f32 / viewport.width * 2.0 - 1.0, cursor[1] as f32 / viewport.height * 2.0 - 1.0];
        let before = self.half(viewport);
        let anchor = [self.centre[0] + ndc[0] * before[0], self.centre[1] + ndc[1] * before[1]];
        self.half_height = (self.half_height * ZOOM_PER_LINE.powf(-lines)).clamp(MIN_HALF_HEIGHT, MAX_HALF_HEIGHT);
        let after = self.half(viewport);
        self.centre = [anchor[0] - ndc[0] * after[0], anchor[1] - ndc[1] * after[1]];
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

    fn point_under(camera: &Camera, viewport: Viewport, cursor: [f64; 2]) -> [f32; 2] {
        let half = camera.half(viewport);
        let ndc =
            [cursor[0] / f64::from(viewport.width) * 2.0 - 1.0, cursor[1] / f64::from(viewport.height) * 2.0 - 1.0];
        [camera.centre[0] + ndc[0] as f32 * half[0], camera.centre[1] + ndc[1] as f32 * half[1]]
    }

    #[rstest]
    fn zoom_keeps_the_point_under_the_cursor_fixed(
        #[values((900, 900), (1600, 600), (500, 1000))] size: (u32, u32),
        #[values((0.0, 0.0), (0.5, 0.5), (1.0, 0.25), (0.8, 0.9))] cursor: (f64, f64),
        #[values(-3.0, 1.0, 5.0)] lines: f32,
    ) {
        let viewport = Viewport::new(size.0, size.1);
        let cursor = [cursor.0 * f64::from(size.0), cursor.1 * f64::from(size.1)];
        let mut camera = Camera { centre: [0.3, 0.7], half_height: 0.2 };
        let before = point_under(&camera, viewport, cursor);
        camera.zoom(lines, cursor, viewport);
        let after = point_under(&camera, viewport, cursor);
        check!(close(before[0], after[0]));
        check!(close(before[1], after[1]));
        check!((camera.half_height - 0.2).abs() > 1e-3);
    }

    #[rstest]
    #[case::zoom_in(100.0, MIN_HALF_HEIGHT)]
    #[case::zoom_out(-100.0, MAX_HALF_HEIGHT)]
    fn zoom_stops_at_the_limits(#[case] lines: f32, #[case] limit: f32) {
        let mut camera = Camera::default();
        camera.zoom(lines, [100.0, 200.0], Viewport::new(900, 900));
        check!(camera.half_height.total_cmp(&limit).is_eq());
    }

    #[rstest]
    #[case(900, 900)]
    #[case(1600, 600)]
    #[case(500, 1000)]
    fn panning_by_the_window_size_moves_the_centre_by_the_visible_extent(#[case] width: u32, #[case] height: u32) {
        let viewport = Viewport::new(width, height);
        let mut camera = Camera { centre: [0.4, 0.6], half_height: 0.3 };
        let visible = camera.half(viewport).map(|h| 2.0 * h);
        camera.pan([f64::from(width), f64::from(height)], viewport);
        check!(close(camera.centre[0], 0.4 - visible[0]));
        check!(close(camera.centre[1], 0.6 - visible[1]));
    }

    #[test]
    fn reset_restores_the_default_view() {
        let viewport = Viewport::new(1280, 720);
        let mut camera = Camera::default();
        camera.zoom(4.0, [300.0, 100.0], viewport);
        camera.pan([50.0, -20.0], viewport);
        camera.reset();
        check!(camera.view(viewport) == Camera::default().view(viewport));
    }
}
