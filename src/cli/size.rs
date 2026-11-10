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
