//! Names, groups and ranges for the shader tuning slots, so the menu and the settings file can
//! speak of `ball_spin` instead of slot 27.

use strum::{Display, VariantArray};

use crate::params::PARAM_SLOTS;

/// Where a tuning parameter is shown in the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, VariantArray)]
pub enum Group {
    Obstacle,
    Eddies,
    Wake,
    Ball,
    Look,
    Debug,
}

/// One tuning slot. Its value is an offset added to a constant built into the shader, so 0 is
/// the picture as designed.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub slot: usize,
    /// The name in the settings file and on the command line.
    pub key: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub min: f32,
    pub max: f32,
    pub help: &'static str,
}

const fn spec(
    slot: usize,
    key: &'static str,
    label: &'static str,
    group: Group,
    min: f32,
    max: f32,
    help: &'static str,
) -> ParamSpec {
    ParamSpec { slot, key, label, group, min, max, help }
}

/// Every slot the shaders read, in slot order. Slot 0 is unused.
pub const CATALOG: [ParamSpec; PARAM_SLOTS - 1] = [
    spec(1, "obstacle_scale", "Obstacle size", Group::Obstacle, -0.8, 1.0, "Flow obstacle radius, relative."),
    spec(
        2,
        "obstacle_shift_x",
        "Obstacle shift across",
        Group::Obstacle,
        -1.0,
        1.0,
        "Obstacle centre shift across the ball, in ball radii.",
    ),
    spec(
        3,
        "obstacle_shift_y",
        "Obstacle shift down",
        Group::Obstacle,
        -1.0,
        1.0,
        "Obstacle centre shift down the ball, in ball radii.",
    ),
    spec(4, "eddy_amp_large", "Large eddy strength", Group::Eddies, -1.4, 2.0, "Strength of the large eddies."),
    spec(5, "eddy_amp_small", "Small eddy strength", Group::Eddies, -0.4, 1.0, "Strength of the small eddies."),
    spec(6, "sinuous_amp", "Sinuous swing strength", Group::Eddies, -0.5, 1.5, "Strength of the slow sinuous swing."),
    spec(7, "glare_gain", "Glare brightness", Group::Look, -1.0, 2.0, "Brightness of the glare round the highlights."),
    spec(8, "eddy_len_x", "Large eddy length along", Group::Eddies, -6.0, 12.0, "Large eddy length along the flow."),
    spec(9, "eddy_len_y", "Large eddy length across", Group::Eddies, -9.0, 12.0, "Large eddy length across the flow."),
    spec(10, "eddy_len_small", "Small eddy size", Group::Eddies, -4.0, 8.0, "Small eddy size."),
    spec(11, "reflection_blur", "Reflection blur", Group::Ball, -3.0, 8.0, "Blur of the liquid mirrored in the ball."),
    spec(
        12,
        "eddy_fade_start",
        "Eddy fade start",
        Group::Eddies,
        -10.0,
        10.0,
        "Where eddies begin fading out downstream.",
    ),
    spec(
        13,
        "eddy_fade_end",
        "Eddy fade end",
        Group::Eddies,
        -10.0,
        20.0,
        "Where eddies are gone and the pattern only drifts.",
    ),
    spec(
        14,
        "eddy_rise_end",
        "Eddy rise end",
        Group::Eddies,
        -6.0,
        10.0,
        "Where eddies reach full strength downstream of the ball.",
    ),
    spec(15, "bounce_tint", "Bounce tint strength", Group::Ball, -0.2, 1.0, "Liquid colour bounced onto the ball."),
    spec(16, "band_edge_tone", "Band edge tone", Group::Look, -0.8, 0.5, "Tone of the lit edge of a band."),
    spec(17, "band_body_tone", "Band body tone", Group::Look, -0.4, 0.5, "Tone of the body of a band."),
    spec(
        18,
        "ball_shadow",
        "Ball shadow darkness",
        Group::Ball,
        -0.2,
        0.7,
        "Darkness of the ball's shadow on the liquid.",
    ),
    spec(
        19,
        "core_noise_damp",
        "Stream core noise",
        Group::Eddies,
        -0.1,
        0.8,
        "Eddy noise left along the stream core.",
    ),
    spec(
        20,
        "ball_reflection",
        "Ball reflection strength",
        Group::Ball,
        -0.15,
        0.8,
        "Strength of the liquid mirrored in the ball.",
    ),
    spec(21, "wake_deficit", "Wake slowdown", Group::Wake, -0.5, 1.0, "How much the liquid slows behind the ball."),
    spec(22, "wake_width", "Wake width", Group::Wake, -8.0, 20.0, "Width of the slow wake."),
    spec(23, "pool_flare", "Pool flare", Group::Obstacle, -0.7, 1.5, "How far the stream flares round the obstacle."),
    spec(
        24,
        "stream_half_width",
        "Stream half width",
        Group::Obstacle,
        -0.7,
        2.0,
        "Half width of the coloured stream.",
    ),
    spec(
        25,
        "fat_band_flatten",
        "Fat band flattening",
        Group::Look,
        -1.0,
        1.0,
        "Flattens airbrushed shading of fat bands.",
    ),
    spec(
        26,
        "wake_start_shift",
        "Wake start shift",
        Group::Wake,
        -0.6,
        3.0,
        "Moves the wake onset downstream, in obstacle radii.",
    ),
    spec(27, "ball_spin", "Ball spin rate", Group::Ball, -1.0, 1.0, "Turning rate of the ball."),
    spec(28, "band_width_jitter", "Band width jitter", Group::Look, -0.05, 0.3, "Band width variation along the flow."),
    spec(29, "spray_mottle", "Spray mottle", Group::Look, -0.05, 0.3, "Raggedness of the far spray edge."),
    spec(30, "flake_tilt", "Flake tilt spread", Group::Ball, -0.3, 1.0, "Tilt spread of the ball's metallic flakes."),
    spec(
        31,
        "debug_ball_outline",
        "Debug ball outline",
        Group::Debug,
        0.0,
        1.0,
        "Debug toggle: dims the ball, outlines its silhouette.",
    ),
];

/// The spec for the slot named `key`.
pub fn by_key(key: &str) -> Option<&'static ParamSpec> {
    CATALOG.iter().find(|s| s.key == key)
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn every_entry_names_the_slot_the_shaders_read() {
        let common = include_str!("../../../../shaders/common.wgsl");
        for (i, spec) in CATALOG.iter().enumerate() {
            let decl = format!("const P_{} = {};", spec.key.to_ascii_uppercase(), spec.slot);
            check!(common.contains(&decl), "{decl} is missing from common.wgsl");
            check!(spec.slot == i + 1);
            check!(spec.min < spec.max && (spec.min..=spec.max).contains(&0.0));
        }
    }
}
