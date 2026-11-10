use strum::{Display, EnumString, IntoStaticStr, VariantArray};

/// A debug view of the flow field, drawn instead of the shaded picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, IntoStaticStr, VariantArray)]
#[strum(serialize_all = "lowercase")]
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

impl FieldView {
    /// The `mode` override `debug.wgsl` switches on.
    pub(super) const fn mode(self) -> u32 {
        self as u32 + 1
    }

    /// The view after `view` in declaration order, with the shaded picture between the last and the first.
    pub fn cycle(view: Option<Self>) -> Option<Self> {
        match view {
            None => Some(Self::VARIANTS[0]),
            Some(v) => Self::VARIANTS.get(v as usize + 1).copied(),
        }
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
        for _ in 0..=FieldView::VARIANTS.len() {
            view = FieldView::cycle(view);
            seen.push(view);
        }
        let expected: Vec<_> = FieldView::VARIANTS.iter().copied().map(Some).chain([None]).collect();
        check!(seen == expected);
    }

    #[test]
    fn unknown_names_are_rejected() {
        check!("velocity".parse::<FieldView>().ok() == Some(FieldView::Velocity));
        check!("speed".parse::<FieldView>().is_err());
    }
}
