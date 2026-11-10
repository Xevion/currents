use std::path::Path;

use image::imageops::{self, FilterType};
use snafu::ResultExt;

use super::{Error, OpenCoverSnafu};
use crate::cli::View;

const GAP: u32 = 8;

/// The cover image, cropped to a view and set beside a render for comparison.
pub(super) struct Reference {
    cover: image::RgbImage,
}

impl Reference {
    pub(super) fn open(path: &Path) -> Result<Self, Error> {
        let cover = image::open(path).context(OpenCoverSnafu { path })?.to_rgb8();
        Ok(Self { cover })
    }

    /// The cover's crop of `view`, resized to match `rendered`, on the left with `rendered` on the right.
    pub(super) fn side_by_side(&self, view: View, rendered: &image::RgbImage) -> image::RgbImage {
        let (width, height) = rendered.dimensions();
        let cover = &self.cover;
        let scale = cover.width() as f32;
        let [left, top] = view.origin();
        let (cx, cy) = ((left * scale).max(0.0) as u32, (top * scale).max(0.0) as u32);
        let (cw, ch) = ((view.width() * scale) as u32, (view.height() * scale) as u32);
        let crop =
            imageops::crop_imm(cover, cx, cy, cw.min(cover.width() - cx), ch.min(cover.height() - cy)).to_image();
        let crop = imageops::resize(&crop, width, height, FilterType::CatmullRom);
        let mut both = image::RgbImage::new(width * 2 + GAP, height);
        imageops::replace(&mut both, &crop, 0, 0);
        imageops::replace(&mut both, rendered, i64::from(width + GAP), 0);
        both
    }
}
