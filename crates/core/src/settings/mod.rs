//! Everything a person can change about the viewer, in the shape the settings file stores.

pub mod catalog;
pub mod keys;
pub mod toml;

use std::collections::BTreeMap;
use std::fmt;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};

pub use self::catalog::{CATALOG, Group, ParamSpec};
pub use self::keys::{Action, Chord, Keymap};
use crate::params::{PARAM_SLOTS, ParamBlock};
use crate::viewer::Budget;

/// The bounds the menu offers and a loaded file is held to.
pub mod limits {
    use std::ops::RangeInclusive;

    pub const SPEED: RangeInclusive<f32> = 0.0..=4.0;
    pub const SUPERSAMPLING: RangeInclusive<u32> = 1..=4;
    pub const FIELD_STEP: RangeInclusive<u32> = 1..=4;
    /// Frames per second a limited frame rate can be held to.
    pub const FPS_CAP: RangeInclusive<u32> = 10..=240;
    /// Shaded pixels per frame a cap can hold to, from a small preview up to 8K.
    pub const MAX_PIXELS: RangeInclusive<f32> = 100_000.0..=33_177_600.0;
    pub const RENDER_SCALE: RangeInclusive<f32> = 0.3..=1.0;
    pub const EASE_SECS: RangeInclusive<f32> = 0.01..=1.0;
    pub const RETURN_SECS: RangeInclusive<f32> = 0.01..=2.0;
    pub const GIVE: RangeInclusive<f32> = 0.0..=0.5;
    pub const ZOOM_PER_LINE: RangeInclusive<f32> = 1.01..=2.0;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub render: Render,
    pub motion: Motion,
    pub camera: CameraTuning,
    pub wallpaper: Wallpaper,
    pub keys: Keymap,
    pub params: Params,
}

/// How much work each frame does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Render {
    /// Blend each frame with the ones before it, smoothing what one sample a pixel leaves rough.
    pub temporal: bool,
    /// Supersampling factor per axis.
    pub supersampling: u32,
    /// Output pixels per flow field texel; an adaptive budget may coarsen it.
    pub field_step: u32,
    /// Frames per second to hold to; without one, frames follow the display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps_cap: Option<u32>,
    /// Pixels shaded per frame, the picture scaled up to fill the window; without a cap every
    /// window pixel is shaded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pixels: Option<f32>,
    /// Per-axis scale the surface starts at.
    pub render_scale: f32,
    /// Lower the field resolution and the scale while frames run long, and recover with headroom.
    pub adaptive: bool,
}

impl Default for Render {
    fn default() -> Self {
        Self {
            temporal: true,
            supersampling: 1,
            field_step: 1,
            fps_cap: None,
            max_pixels: None,
            render_scale: 1.0,
            adaptive: false,
        }
    }
}

impl Render {
    /// The pixel budget these fields describe; `None` renders every window pixel as it comes.
    pub fn budget(&self) -> Option<Budget> {
        if self.max_pixels.is_none() && !self.adaptive && self.render_scale >= 1.0 {
            return None;
        }
        Some(Budget {
            max_pixels: self.max_pixels.map_or(f64::INFINITY, f64::from),
            scale: f64::from(self.render_scale),
            adaptive: self.adaptive,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Motion {
    /// Flow speed multiplier.
    pub speed: f32,
}

impl Default for Motion {
    fn default() -> Self {
        Self { speed: 0.4 }
    }
}

/// How the view answers dragging and scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraTuning {
    /// Ignore drag, scroll and pinch, so clicks on a desktop never move the picture.
    pub locked: bool,
    /// Seconds in which the shown view closes about two thirds of the way to where input put it.
    pub ease_secs: f32,
    /// Seconds in which a view let go past the edge closes most of the way back.
    pub return_secs: f32,
    /// Furthest a view can be pushed past the cover, as a share of its own size.
    pub give: f32,
    /// Zoom factor per scroll line.
    pub zoom_per_line: f32,
}

impl Default for CameraTuning {
    fn default() -> Self {
        Self { locked: false, ease_secs: 0.08, return_secs: 0.18, give: 0.15, zoom_per_line: 1.15 }
    }
}

/// Behaviour that matters once the viewer sits behind a desktop.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wallpaper {
    /// Stop drawing while the window is fully covered.
    pub pause_when_hidden: bool,
    /// Name the menu key when an unbound key is pressed.
    pub hint: bool,
    /// Holding the right button on the ball opens the menu.
    pub ball_opener: bool,
}

impl Default for Wallpaper {
    fn default() -> Self {
        Self { pause_when_hidden: true, hint: true, ball_opener: true }
    }
}

/// The shader tuning offsets, stored by name.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Params(pub ParamBlock);

impl Params {
    pub const fn get(&self, slot: usize) -> f32 {
        self.0[slot / 4][slot % 4]
    }

    pub const fn get_mut(&mut self, slot: usize) -> &mut f32 {
        &mut self.0[slot / 4][slot % 4]
    }
}

impl Serialize for Params {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(CATALOG.iter().map(|spec| (spec.key, self.get(spec.slot))))
    }
}

impl<'de> Deserialize<'de> for Params {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let named = BTreeMap::<String, f32>::deserialize(d)?;
        let mut params = Self::default();
        for (key, value) in named {
            if let Some(spec) = catalog::by_key(&key) {
                *params.get_mut(spec.slot) = value;
            }
        }
        Ok(params)
    }
}

const _: () = assert!(CATALOG.len() == PARAM_SLOTS - 1);

/// A value a loaded file could not use as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Dotted path of the setting, as in `render.fps_cap`.
    pub path: String,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

fn clamp_into<T: PartialOrd + Copy + fmt::Display>(
    value: &mut T,
    range: &RangeInclusive<T>,
    path: &str,
    issues: &mut Vec<Issue>,
) {
    let held = if *value < *range.start() {
        *range.start()
    } else if *value > *range.end() {
        *range.end()
    } else {
        return;
    };
    issues.push(Issue { path: path.to_owned(), message: format!("{value} is out of range; using {held}") });
    *value = held;
}

impl Settings {
    /// Settings with every optional value present, which shows the file every key it may hold.
    pub(crate) fn schema() -> Self {
        let mut s = Self::default();
        s.render.fps_cap = Some(60);
        s.render.max_pixels = Some(3_686_400.0);
        s
    }

    /// Pulls every value inside its bounds, reporting each one moved.
    pub fn clamp(&mut self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let r = &mut self.render;
        clamp_into(&mut r.supersampling, &limits::SUPERSAMPLING, "render.supersampling", &mut issues);
        clamp_into(&mut r.field_step, &limits::FIELD_STEP, "render.field_step", &mut issues);
        // Nothing or less per frame meant no limit, so it reads as none rather than as a tiny one.
        r.fps_cap = r.fps_cap.filter(|&v| v > 0);
        r.max_pixels = r.max_pixels.filter(|&v| v > 0.0);
        if let Some(fps) = r.fps_cap.as_mut() {
            clamp_into(fps, &limits::FPS_CAP, "render.fps_cap", &mut issues);
        }
        if let Some(pixels) = r.max_pixels.as_mut() {
            clamp_into(pixels, &limits::MAX_PIXELS, "render.max_pixels", &mut issues);
        }
        clamp_into(&mut r.render_scale, &limits::RENDER_SCALE, "render.render_scale", &mut issues);
        clamp_into(&mut self.motion.speed, &limits::SPEED, "motion.speed", &mut issues);
        let c = &mut self.camera;
        clamp_into(&mut c.ease_secs, &limits::EASE_SECS, "camera.ease_secs", &mut issues);
        clamp_into(&mut c.return_secs, &limits::RETURN_SECS, "camera.return_secs", &mut issues);
        clamp_into(&mut c.give, &limits::GIVE, "camera.give", &mut issues);
        clamp_into(&mut c.zoom_per_line, &limits::ZOOM_PER_LINE, "camera.zoom_per_line", &mut issues);
        for spec in &CATALOG {
            let path = format!("params.{}", spec.key);
            clamp_into(self.params.get_mut(spec.slot), &(spec.min..=spec.max), &path, &mut issues);
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;
    use rstest::rstest;

    use super::*;

    #[test]
    fn defaults_sit_inside_their_bounds() {
        check!(Settings::default().clamp().is_empty());
    }

    #[test]
    fn values_past_their_bounds_are_held_at_the_edge_and_reported() {
        let mut settings = Settings::default();
        settings.render.fps_cap = Some(1000);
        *settings.params.get_mut(27) = -50.0;
        let issues = settings.clamp();
        check!(settings.render.fps_cap == Some(*limits::FPS_CAP.end()));
        check!(settings.params.get(27).total_cmp(&CATALOG[26].min).is_eq());
        let paths: Vec<_> = issues.iter().map(|i| i.path.as_str()).collect();
        check!(paths == ["render.fps_cap", "params.ball_spin"]);
    }

    #[rstest]
    #[case::no_frames(Some(0), None)]
    #[case::too_slow(Some(3), Some(*limits::FPS_CAP.start()))]
    #[case::within(Some(60), Some(60))]
    #[case::none(None, None)]
    fn a_frame_rate_limit_of_nothing_means_no_limit(#[case] given: Option<u32>, #[case] held: Option<u32>) {
        let mut settings = Settings::default();
        settings.render.fps_cap = given;
        settings.clamp();
        check!(settings.render.fps_cap == held);
    }

    #[rstest]
    #[case::nothing(Some(0.0), None)]
    #[case::too_few(Some(10.0), Some(*limits::MAX_PIXELS.start()))]
    #[case::within(Some(800_000.0), Some(800_000.0))]
    #[case::too_many(Some(1e9), Some(*limits::MAX_PIXELS.end()))]
    fn a_pixel_cap_is_held_within_what_the_menu_offers(#[case] given: Option<f32>, #[case] held: Option<f32>) {
        let mut settings = Settings::default();
        settings.render.max_pixels = given;
        settings.clamp();
        check!(settings.render.max_pixels == held);
    }

    #[test]
    fn the_schema_sits_inside_its_bounds_too() {
        check!(Settings::schema().clamp().is_empty());
    }

    #[test]
    fn the_default_render_settings_keep_every_window_pixel() {
        check!(Render::default().budget().is_none());
        let capped = Render { max_pixels: Some(1e6), ..Render::default() };
        check!(capped.budget().map(|b| b.max_pixels) == Some(1e6));
    }
}
