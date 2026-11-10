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
