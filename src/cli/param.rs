use std::num::{ParseFloatError, ParseIntError};
use std::str::FromStr;

use snafu::{OptionExt, ResultExt, Snafu};

/// Number of float slots the shaders read tuning parameters from.
pub(super) const PARAM_SLOTS: usize = 32;

/// One `INDEX=VALUE` tuning override.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParamOverride {
    slot: usize,
    value: f32,
}

#[derive(Debug, Snafu)]
pub(crate) enum Error {
    #[snafu(display("expected INDEX=VALUE"))]
    Shape,
    #[snafu(display("index: {source}"))]
    Slot { source: ParseIntError },
    #[snafu(display("index must be below {PARAM_SLOTS}"))]
    Range,
    #[snafu(display("value: {source}"))]
    Value { source: ParseFloatError },
}

impl FromStr for ParamOverride {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (slot, value) = s.split_once('=').context(ShapeSnafu)?;
        let slot: usize = slot.parse().context(SlotSnafu)?;
        if slot >= PARAM_SLOTS {
            return RangeSnafu.fail();
        }
        Ok(Self { slot, value: value.parse().context(ValueSnafu)? })
    }
}

impl ParamOverride {
    /// Writes each override into its slot of an otherwise zeroed block; later ones win.
    pub(super) fn pack(overrides: &[Self]) -> [[f32; 4]; PARAM_SLOTS / 4] {
        let mut out = [[0.0; 4]; PARAM_SLOTS / 4];
        for o in overrides {
            out[o.slot / 4][o.slot % 4] = o.value;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::plain("3=0.5", 3, 0.5)]
    #[case::negative_value("0=-2", 0, -2.0)]
    #[case::last_slot("31=1", 31, 1.0)]
    fn parses_index_and_value(#[case] input: &str, #[case] slot: usize, #[case] value: f32) {
        assert!(let Ok(parsed) = input.parse::<ParamOverride>());
        check!(parsed.slot == slot);
        check!(parsed.value.total_cmp(&value).is_eq());
    }

    #[rstest]
    #[case::no_equals("5", "shape")]
    #[case::first_slot_past_the_end("32=1", "range")]
    #[case::non_numeric_slot("x=1", "slot")]
    #[case::negative_slot("-1=1", "slot")]
    #[case::bad_value("1=y", "value")]
    #[case::empty_value("1=", "value")]
    fn rejects_malformed_overrides(#[case] input: &str, #[case] kind: &str) {
        assert!(let Err(error) = input.parse::<ParamOverride>());
        let got = match error {
            Error::Shape => "shape",
            Error::Slot { .. } => "slot",
            Error::Range => "range",
            Error::Value { .. } => "value",
        };
        check!(got == kind);
    }

    #[test]
    fn pack_puts_each_override_in_its_slot_and_the_last_one_wins() {
        let overrides: Vec<ParamOverride> = ["0=1", "5=2", "5=3", "31=4"].iter().map(|s| s.parse().unwrap()).collect();
        let mut expected = [[0.0; 4]; PARAM_SLOTS / 4];
        expected[0][0] = 1.0;
        expected[1][1] = 3.0;
        expected[7][3] = 4.0;
        check!(ParamOverride::pack(&overrides) == expected);
    }
}
