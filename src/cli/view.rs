use std::num::ParseFloatError;
use std::str::FromStr;

use snafu::{ResultExt, Snafu};

/// A rectangle in cover coordinates, where the cover square is `0..1` on both axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl View {
    const FULL: Self = Self::new(0.0, 0.0, 1.0, 1.0);

    /// A named view given as a pixel rectangle of the 1500 pixel cover.
    const fn pixels(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self::new(x / 1500.0, y / 1500.0, (x + w) / 1500.0, (y + h) / 1500.0)
    }

    const NAMED: [(&'static str, Self); 11] = [
        ("full", Self::FULL),
        ("wide", Self::new(-0.3889, 0.0, 1.3889, 1.0)),
        ("a", Self::pixels(600.0, 740.0, 420.0, 420.0)),
        ("b", Self::pixels(400.0, 760.0, 400.0, 400.0)),
        ("c", Self::pixels(900.0, 880.0, 450.0, 450.0)),
        ("d", Self::pixels(1000.0, 1200.0, 340.0, 300.0)),
        ("e", Self::pixels(280.0, 375.0, 375.0, 375.0)),
        ("f", Self::pixels(470.0, 1125.0, 400.0, 375.0)),
        ("g", Self::pixels(1100.0, 0.0, 400.0, 400.0)),
        ("h", Self::pixels(1125.0, 750.0, 375.0, 375.0)),
        ("user", Self::pixels(330.0, 560.0, 900.0, 755.0)),
    ];

    const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// The rectangle `half` extents away from `centre` on each side.
    pub(crate) fn from_centre(centre: [f32; 2], half: [f32; 2]) -> Self {
        Self::new(centre[0] - half[0], centre[1] - half[1], centre[0] + half[0], centre[1] + half[1])
    }

    pub(crate) fn width(self) -> f32 {
        self.x1 - self.x0
    }

    pub(crate) fn height(self) -> f32 {
        self.y1 - self.y0
    }

    pub(crate) fn aspect(self) -> f32 {
        self.width() / self.height()
    }

    /// Left and top edge, in cover coordinates.
    pub(crate) fn origin(self) -> [f32; 2] {
        [self.x0, self.y0]
    }

    /// The `[x0, y0, x1, y1]` layout the shaders read.
    pub(crate) fn to_array(self) -> [f32; 4] {
        [self.x0, self.y0, self.x1, self.y1]
    }
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("expected a view name or x0,y0,x1,y1"))]
    Shape,
    #[snafu(display("`{part}` is not a number: {source}"))]
    Coord { part: String, source: ParseFloatError },
}

impl FromStr for View {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((_, view)) = Self::NAMED.iter().find(|(name, _)| *name == s) {
            return Ok(*view);
        }
        let parts: Vec<&str> = s.split(',').collect();
        let [x0, y0, x1, y1] = parts[..] else { return ShapeSnafu.fail() };
        let coord = |part: &str| part.parse().context(CoordSnafu { part });
        Ok(Self::new(coord(x0)?, coord(y0)?, coord(x1)?, coord(y1)?))
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::named_pixel_view("a", [0.4, 0.493_333_3, 0.68, 0.773_333_3])]
    #[case::negatives("-0.25,-0.5,1.5,2", [-0.25, -0.5, 1.5, 2.0])]
    #[case::plain("0.1,0.2,0.3,0.4", [0.1, 0.2, 0.3, 0.4])]
    fn parses_names_and_coordinates(#[case] input: &str, #[case] expected: [f32; 4]) {
        assert!(let Ok(view) = input.parse::<View>());
        check!(view.to_array().iter().zip(expected).all(|(got, want)| (got - want).abs() < 1e-6));
    }

    #[rstest]
    #[case::unknown_name("nope", true)]
    #[case::empty("", true)]
    #[case::three_parts("1,2,3", true)]
    #[case::five_parts("1,2,3,4,5", true)]
    #[case::not_a_number("1,2,x,4", false)]
    fn rejects_malformed_views(#[case] input: &str, #[case] wrong_shape: bool) {
        assert!(let Err(error) = input.parse::<View>());
        check!(matches!(error, Error::Shape) == wrong_shape);
    }
}
