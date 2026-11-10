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
        let mut both = image::RgbImage::new(width * 2 + GAP, height);
        if let Some(CropRect { x, y, width: cw, height: ch }) = crop_rect(view, cover.width(), cover.height()) {
            let crop = imageops::crop_imm(cover, x, y, cw, ch).to_image();
            let crop = imageops::resize(&crop, width, height, FilterType::CatmullRom);
            imageops::replace(&mut both, &crop, 0, 0);
        }
        imageops::replace(&mut both, rendered, i64::from(width + GAP), 0);
        both
    }
}

/// A rectangle of cover pixels.
#[derive(Debug, PartialEq, Eq)]
struct CropRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// The cover pixels `view` shows, with the cover scaled to its width on both axes.
///
/// The start clamps to the cover's top left edge and the extent stays the view's own size, cut off at
/// the cover's far edges. `None` when the view misses the cover or has no area.
fn crop_rect(view: View, cover_width: u32, cover_height: u32) -> Option<CropRect> {
    let scale = cover_width as f32;
    let [left, top] = view.origin();
    if (left + view.width()) * scale <= 0.0 || (top + view.height()) * scale <= 0.0 {
        return None;
    }
    let (x, y) = ((left * scale).max(0.0) as u32, (top * scale).max(0.0) as u32);
    let (width, height) = ((view.width() * scale) as u32, (view.height() * scale) as u32);
    let rect = CropRect {
        x,
        y,
        width: width.min(cover_width.saturating_sub(x)),
        height: height.min(cover_height.saturating_sub(y)),
    };
    (rect.width > 0 && rect.height > 0).then_some(rect)
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    const COVER: u32 = 1000;

    #[rstest]
    #[case::inside("0.25,0.25,0.75,0.5", Some((250, 250, 500, 250)))]
    #[case::off_left_and_top("-0.5,-0.25,0.5,0.5", Some((0, 0, 1000, 750)))]
    #[case::off_right_and_bottom("0.75,0.5,1.5,1.25", Some((750, 500, 250, 500)))]
    #[case::entirely_right("1.5,0,2,1", None)]
    #[case::entirely_below("0,1.2,1,2", None)]
    #[case::entirely_left("-2,0,-1,1", None)]
    #[case::entirely_above("0,-2,1,-1", None)]
    #[case::no_area("0.5,0.5,0.5,0.75", None)]
    fn crop_follows_the_view_and_stays_on_the_cover(
        #[case] view: &str,
        #[case] expected: Option<(u32, u32, u32, u32)>,
    ) {
        let view: View = view.parse().unwrap();
        let expected = expected.map(|(x, y, width, height)| CropRect { x, y, width, height });
        check!(crop_rect(view, COVER, COVER) == expected);
    }
}
