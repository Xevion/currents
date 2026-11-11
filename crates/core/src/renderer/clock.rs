//! The shaders' clock. A float of flow seconds loses the precision the trace needs within a
//! day, so the shaders count from a restart every `EPOCH` seconds instead, and are told where
//! each pattern that drifts with time had got to at the restart, worked out here in full.

use std::f64::consts::TAU;

use crate::params::ParamBlock;

/// Seconds of flow between restarts of the shaders' clock.
pub(super) const EPOCH: f64 = 1600.0;
/// How far a restart moves every flow label, which the patterns drawn from labels repeat over.
pub(super) const LABEL_STEP: f64 = FLOW_SPEED * EPOCH;

// The shaders' rates of drift and the periods each drift repeats over.
const FLOW_SPEED: f64 = 4.0;
const NOISE_PERIOD: f64 = 64.0;
const EDDY_LEN_X: f64 = 9.0;
const EDDY_LEN_SMALL: f64 = 5.5;
const LARGE_EDDY_CHURN: f64 = 0.04;
const SMALL_EDDY_CHURN: f64 = 0.05;
const SINUOUS_LEN: f64 = 26.0;
const BALL_SPIN: f64 = 0.08;
const GLARE_RATE: f64 = 0.9;

// Tuning slots that change a rate of drift.
const P_EDDY_LEN_X: usize = 8;
const P_EDDY_LEN_SMALL: usize = 10;
const P_BALL_SPIN: usize = 27;

/// The shaders' clock at some time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Clock {
    /// Restarts before then.
    pub(super) epoch: f32,
    /// Seconds since the last restart.
    pub(super) time: f32,
    /// Where the large eddy noise (two axes), the small (one) and the sinuous swing had
    /// drifted to at the restart.
    pub(super) eddy_drift: [f32; 4],
    /// The ball's turn at the restart.
    pub(super) spin: f32,
    /// The glare's flicker at the restart.
    pub(super) glare_drift: f32,
}

impl Clock {
    pub(super) fn at(time: f64, params: &ParamBlock) -> Self {
        let epoch = epoch(time);
        let start = epoch * EPOCH;
        let param = |slot: usize| f64::from(params[slot / 4][slot % 4]);
        let wrap = |drift: f64, period: f64| drift.rem_euclid(period) as f32;
        let carried_large = -FLOW_SPEED / (EDDY_LEN_X + param(P_EDDY_LEN_X));
        let carried_small = -FLOW_SPEED / (EDDY_LEN_SMALL + param(P_EDDY_LEN_SMALL));
        Self {
            epoch: epoch as f32,
            time: (time - start) as f32,
            eddy_drift: [
                wrap(carried_large * start, NOISE_PERIOD),
                wrap(LARGE_EDDY_CHURN * start, NOISE_PERIOD),
                wrap((carried_small + SMALL_EDDY_CHURN) * start, NOISE_PERIOD),
                wrap(FLOW_SPEED * start, SINUOUS_LEN),
            ],
            spin: wrap((BALL_SPIN + param(P_BALL_SPIN)) * start, TAU),
            glare_drift: wrap(GLARE_RATE * start, NOISE_PERIOD),
        }
    }
}

/// Restarts of the clock before `time`.
pub(super) fn epoch(time: f64) -> f64 {
    (time / EPOCH).floor()
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;
    use crate::shader_check::declared;

    #[rstest]
    #[case("EPOCH", EPOCH)]
    #[case("FLOW_SPEED", FLOW_SPEED)]
    #[case("NOISE_PERIOD", NOISE_PERIOD)]
    #[case("EDDY_LEN_X", EDDY_LEN_X)]
    #[case("EDDY_LEN_SMALL", EDDY_LEN_SMALL)]
    #[case("LARGE_EDDY_CHURN", LARGE_EDDY_CHURN)]
    #[case("SMALL_EDDY_CHURN", SMALL_EDDY_CHURN)]
    #[case("SINUOUS_LEN", SINUOUS_LEN)]
    #[case("BALL_SPIN", BALL_SPIN)]
    #[case("GLARE_RATE", GLARE_RATE)]
    #[case("P_EDDY_LEN_X", P_EDDY_LEN_X as f64)]
    #[case("P_EDDY_LEN_SMALL", P_EDDY_LEN_SMALL as f64)]
    #[case("P_BALL_SPIN", P_BALL_SPIN as f64)]
    fn constants_match_the_embedded_shaders(#[case] name: &str, #[case] value: f64) {
        check!(declared(name) == Some(value));
    }

    /// The large eddy noise's place along the flow, as the shaders work it out from a clock.
    fn large_eddy_drift(clock: Clock) -> f64 {
        let rate = -FLOW_SPEED / EDDY_LEN_X;
        (f64::from(clock.eddy_drift[0]) + rate * f64::from(clock.time)).rem_euclid(NOISE_PERIOD)
    }

    #[rstest]
    #[case::an_hour_in(3600.0)]
    #[case::a_year_in(365.0 * 86400.0)]
    fn a_restart_carries_the_drift_on_where_it_was(#[case] seconds: f64) {
        let params = [[0.0; 4]; 8];
        // the last moment before a restart and the first after
        let restart = (epoch(seconds) + 1.0) * EPOCH;
        let before = Clock::at(restart - 1e-3, &params);
        let after = Clock::at(restart, &params);
        check!(after.epoch.total_cmp(&(before.epoch + 1.0)).is_eq() && after.time.total_cmp(&0.0).is_eq());
        let half = 0.5 * NOISE_PERIOD;
        let gap = (large_eddy_drift(after) - large_eddy_drift(before) + half).rem_euclid(NOISE_PERIOD) - half;
        check!((gap + 1e-3 * FLOW_SPEED / EDDY_LEN_X).abs() < 1e-4);
    }
}
