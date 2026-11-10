use std::fmt;
use std::str::FromStr;

use snafu::Snafu;

/// A debug view of the flow field, drawn instead of the shaded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldView {
    /// The stripe coordinate: a sawtooth per period, integer contours, the stream in red.
    Line,
    /// The label riding with the liquid along each band, as hue.
    Label,
    /// Screen velocity: direction as hue, speed as brightness, with arrows.
    Velocity,
    /// Stripe periods per output pixel; magenta past two pixels per period.
    Density,
}

#[derive(Debug, Snafu)]
#[snafu(display("expected one of line, label, velocity, density"))]
pub struct UnknownView;

impl FieldView {
    pub const ALL: [Self; 4] = [Self::Line, Self::Label, Self::Velocity, Self::Density];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Label => "label",
            Self::Velocity => "velocity",
            Self::Density => "density",
        }
    }

    /// The `mode` override `debug.wgsl` switches on.
    pub(super) const fn mode(self) -> u32 {
        self as u32 + 1
    }

    /// The view after `view` in `ALL`, with the shaded picture between the last and the first.
    pub fn cycle(view: Option<Self>) -> Option<Self> {
        match view {
            None => Some(Self::ALL[0]),
            Some(v) => Self::ALL.get(v as usize + 1).copied(),
        }
    }
}

impl fmt::Display for FieldView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for FieldView {
    type Err = UnknownView;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|v| v.name() == s).ok_or(UnknownView)
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn cycling_visits_every_view_then_the_shaded_picture() {
        let mut view = None;
        let mut seen = Vec::new();
        for _ in 0..=FieldView::ALL.len() {
            view = FieldView::cycle(view);
            seen.push(view);
        }
        let expected: Vec<_> = FieldView::ALL.into_iter().map(Some).chain([None]).collect();
        check!(seen == expected);
    }

    #[test]
    fn unknown_names_are_rejected() {
        check!("velocity".parse::<FieldView>().ok() == Some(FieldView::Velocity));
        check!("speed".parse::<FieldView>().is_err());
    }
}
