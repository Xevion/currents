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
