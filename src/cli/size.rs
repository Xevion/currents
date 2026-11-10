use std::num::ParseIntError;
use std::str::FromStr;

use snafu::{OptionExt, ResultExt, Snafu};

use super::View;

/// Width and height of the rendered image in pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutputSize {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("expected WxH"))]
    Shape,
    #[snafu(display("width: {source}"))]
    Width { source: ParseIntError },
    #[snafu(display("height: {source}"))]
    Height { source: ParseIntError },
}

impl FromStr for OutputSize {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (width, height) = s.split_once('x').context(ShapeSnafu)?;
        Ok(Self { width: width.parse().context(WidthSnafu)?, height: height.parse().context(HeightSnafu)? })
    }
}

impl OutputSize {
    /// 800 pixels tall at the view's aspect ratio.
    pub(crate) fn for_view(view: View) -> Self {
        Self { width: (800.0 * view.aspect()).round() as u32, height: 800 }
    }

    /// The size with both axes multiplied by `factor`.
    pub(crate) fn scaled(self, factor: u32) -> Self {
        Self { width: self.width * factor, height: self.height * factor }
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::square("64x64", 64, 64)]
    #[case::landscape("800x600", 800, 600)]
    fn parses_width_and_height(#[case] input: &str, #[case] width: u32, #[case] height: u32) {
        assert!(let Ok(size) = input.parse::<OutputSize>());
        check!((size.width, size.height) == (width, height));
    }

    #[rstest]
    #[case::no_separator("64", "shape")]
    #[case::capital_separator("64X32", "shape")]
    #[case::missing_height("64x", "height")]
    #[case::missing_width("x32", "width")]
    #[case::non_numeric_width("axb", "width")]
    #[case::negative_height("64x-3", "height")]
    #[case::three_parts("64x32x1", "height")]
    fn rejects_malformed_sizes(#[case] input: &str, #[case] kind: &str) {
        assert!(let Err(error) = input.parse::<OutputSize>());
        let got = match error {
            Error::Shape => "shape",
            Error::Width { .. } => "width",
            Error::Height { .. } => "height",
        };
        check!(got == kind);
    }

    #[rstest]
    #[case::three_to_two("0,0,1.5,1", 1200)]
    #[case::rounds_to_nearest("0,0,1.3333,1", 1067)]
    #[case::portrait("0,0,0.5,1", 400)]
    fn default_size_is_800_tall_at_the_views_aspect(#[case] view: &str, #[case] width: u32) {
        let view: View = view.parse().unwrap();
        let size = OutputSize::for_view(view);
        check!((size.width, size.height) == (width, 800));
    }
}
