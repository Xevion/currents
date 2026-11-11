//! The settings a wallpaper host shows in its own property panel, and how what it sends back
//! becomes settings. Hosts list properties flat, so this is a short pick of the menu's settings.

use strum::{Display, EnumString, VariantArray};

use crate::settings::catalog::{CATALOG, Group};
use crate::settings::toml::Value;
use crate::settings::{Settings, limits};

/// A wallpaper host that runs the web viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Display, EnumString, VariantArray)]
#[strum(serialize_all = "lowercase")]
pub enum Host {
    /// Wallpaper Engine, which sets its own frame rate limit and sends it as `fps`.
    #[strum(serialize = "we")]
    WallpaperEngine,
    Lively,
}

/// The control a host draws for a property, with the value it starts at.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Slider { min: f64, max: f64, step: f64, value: f64 },
    Toggle { value: bool },
}

/// One property in a host's panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    /// The host's name for it: lowercase, starting with a letter, as both hosts require.
    pub key: String,
    pub label: String,
    pub kind: Kind,
}

/// The key a host sends for the frame rate, both Wallpaper Engine's own and Lively's slider.
const FPS: &str = "fps";
/// Lively's switch for the frame rate limit; Wallpaper Engine has its own.
const FPS_LIMIT: &str = "fps_limit";
/// The frame rate a limit starts at when switched on.
const FPS_DEFAULT: u32 = 30;

fn key_of(path: &str) -> String {
    path.replace('.', "_")
}

fn slider(path: &str, label: &str, range: (f64, f64), step: f64, value: f64) -> Prop {
    Prop { key: key_of(path), label: label.to_owned(), kind: Kind::Slider { min: range.0, max: range.1, step, value } }
}

fn toggle(path: &str, label: &str, value: bool) -> Prop {
    Prop { key: key_of(path), label: label.to_owned(), kind: Kind::Toggle { value } }
}

/// The settings each host shows; the setting a property changes is its key with `_` for `.`,
/// apart from the frame rate pair.
const PATHS: [&str; 6] = [
    "motion.speed",
    "render.temporal",
    "render.supersampling",
    "render.render_scale",
    "camera.locked",
    "wallpaper.hint",
];

/// `x` as the f64 that reads the same, so 0.4 is not written as 0.4000000059604645.
fn wide(x: f32) -> f64 {
    x.to_string().parse().unwrap_or_else(|_| f64::from(x))
}

/// A slider step that splits `range` into about a hundred steps of a round size.
fn step_for(range: (f64, f64)) -> f64 {
    10f64.powf(((range.1 - range.0) / 100.0).log10().floor())
}

/// The properties `host` shows, starting at the values a wallpaper wants: the view locked, so
/// clicks on the desktop never move it, and no hint about keys a host may never pass on.
pub fn props(host: Host) -> Vec<Prop> {
    let d = Settings::default();
    let range = |r: &std::ops::RangeInclusive<f32>| (wide(*r.start()), wide(*r.end()));
    let whole = |r: &std::ops::RangeInclusive<u32>| (f64::from(*r.start()), f64::from(*r.end()));
    let mut props = vec![
        slider(PATHS[0], "Speed", range(&limits::SPEED), 0.05, wide(d.motion.speed)),
        toggle(PATHS[1], "Smooth across frames", d.render.temporal),
        slider(PATHS[2], "Supersampling", whole(&limits::SUPERSAMPLING), 1.0, f64::from(d.render.supersampling)),
        slider(PATHS[3], "Render scale", range(&limits::RENDER_SCALE), 0.05, wide(d.render.render_scale)),
        toggle(PATHS[4], "Lock the view", true),
        toggle(PATHS[5], "Hint at the menu key", false),
    ];
    if host == Host::Lively {
        props.push(Prop {
            key: FPS_LIMIT.to_owned(),
            label: "Limit the frame rate".to_owned(),
            kind: Kind::Toggle { value: false },
        });
        props.push(Prop {
            key: FPS.to_owned(),
            label: "Frame rate limit".to_owned(),
            kind: Kind::Slider {
                min: whole(&limits::FPS_CAP).0,
                max: whole(&limits::FPS_CAP).1,
                step: 1.0,
                value: f64::from(FPS_DEFAULT),
            },
        });
    }
    for spec in CATALOG.iter().filter(|s| s.group == Group::Look || s.key == "ball_spin") {
        let r = (wide(spec.min), wide(spec.max));
        let path = format!("params.{}", spec.key);
        props.push(slider(&path, spec.label, r, step_for(r), wide(d.params.get(spec.slot))));
    }
    props
}

/// Turns what a host sends into setting changes. It keeps the frame rate pair, since Lively
/// sends the switch and the rate as two properties in no fixed order.
#[derive(Debug, Default)]
pub struct Receiver {
    /// The rate last sent, if any.
    fps: Option<u32>,
    /// The switch last sent; a host without one sends only a rate, where 0 means no limit.
    limit: Option<bool>,
}

/// A setting to change, by dotted path; `None` switches an optional one off.
pub type Assignment = (String, Option<Value>);

impl Receiver {
    /// The settings that `value` for the host's property `key` changes.
    pub fn receive(&mut self, key: &str, value: Value) -> Result<Vec<Assignment>, String> {
        // Hosts send every number as a float; a whole one has to reach a whole-number setting.
        let value = match value.as_float() {
            Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Value::from(f as i64),
            _ => value,
        };
        match key {
            FPS => {
                let rate = value.as_integer().or_else(|| value.as_float().map(|f| f.round() as i64));
                let rate = rate.ok_or_else(|| format!("{FPS}: expected a number, not {value}"))?;
                self.fps = Some(u32::try_from(rate).unwrap_or(0));
                Ok(vec![self.frame_rate()])
            },
            FPS_LIMIT => {
                self.limit = Some(value.as_bool().ok_or_else(|| format!("{FPS_LIMIT}: expected true or false"))?);
                Ok(vec![self.frame_rate()])
            },
            _ => {
                let path = PATHS
                    .iter()
                    .map(|p| (*p).to_owned())
                    .chain(CATALOG.iter().map(|s| format!("params.{}", s.key)))
                    .find(|p| key_of(p) == key)
                    .ok_or_else(|| format!("{key}: not a property"))?;
                Ok(vec![(path, Some(value))])
            },
        }
    }

    fn frame_rate(&self) -> Assignment {
        let cap = match self.limit {
            Some(false) => None,
            Some(true) => Some(self.fps.unwrap_or(FPS_DEFAULT)),
            // Wallpaper Engine's own limit, which uses 0 for none.
            None => self.fps.filter(|&fps| fps > 0),
        };
        ("render.fps_cap".to_owned(), cap.map(|c| Value::from(i64::from(c))))
    }
}

#[cfg(test)]
mod tests {
    use assert2::{assert, check};
    use rstest::rstest;

    use super::*;
    use crate::settings::toml::assign;

    /// What the settings end up as after `host` sends every property at the value it starts at.
    fn after_load(host: Host) -> Settings {
        let mut receiver = Receiver::default();
        let mut settings = Settings::default();
        for prop in props(host) {
            let value = match prop.kind {
                Kind::Slider { value, .. } => Value::from(value),
                Kind::Toggle { value } => Value::from(value),
            };
            for (path, value) in receiver.receive(&prop.key, value).unwrap() {
                settings = assign(&settings, &path, value).unwrap().0;
            }
        }
        settings
    }

    #[rstest]
    fn every_property_starts_at_a_value_the_setting_takes_as_is(
        #[values(Host::WallpaperEngine, Host::Lively)] host: Host,
    ) {
        let mut receiver = Receiver::default();
        for prop in props(host) {
            let (value, min, max) = match prop.kind {
                Kind::Slider { value, min, max, step } => {
                    check!(step > 0.0 && step <= max - min, "{}", prop.key);
                    (Value::from(value), Value::from(min), Value::from(max))
                },
                Kind::Toggle { value } => (Value::from(value), Value::from(!value), Value::from(value)),
            };
            for value in <[Value; 3]>::from((value, min, max)) {
                assert!(let Ok(changes) = receiver.receive(&prop.key, value.clone()), "{}", prop.key);
                for (path, value) in changes {
                    assert!(let Ok((_, issues)) = assign(&Settings::default(), &path, value), "{path}");
                    check!(issues.is_empty(), "{path}: {issues:?}");
                }
            }
        }
    }

    #[test]
    fn a_wallpaper_starts_locked_and_without_the_key_hint() {
        let settings = after_load(Host::Lively);
        check!(settings.camera.locked);
        check!(!settings.wallpaper.hint);
        check!(settings.render.fps_cap.is_none());
        check!(settings.params == Settings::default().params);
    }

    #[rstest]
    #[case::we_unlimited(&[("fps", 0)], None)]
    #[case::we_limited(&[("fps", 24)], Some(24))]
    #[case::lively_on_then_rate(&[("fps_limit", 1), ("fps", 45)], Some(45))]
    #[case::lively_rate_then_on(&[("fps", 45), ("fps_limit", 1)], Some(45))]
    #[case::lively_on_alone(&[("fps_limit", 1)], Some(FPS_DEFAULT))]
    #[case::lively_off_keeps_no_limit(&[("fps", 45), ("fps_limit", 0)], None)]
    fn the_frame_rate_follows_the_host(#[case] sent: &[(&str, i64)], #[case] cap: Option<u32>) {
        let mut receiver = Receiver::default();
        let mut last = None;
        for &(key, n) in sent {
            let value = if key == FPS_LIMIT { Value::from(n == 1) } else { Value::from(n) };
            last = receiver.receive(key, value).unwrap().pop();
        }
        let last = last.map(|(path, value)| (path, value.map(|v| v.to_string())));
        check!(last == Some(("render.fps_cap".to_owned(), cap.map(|c| c.to_string()))));
    }

    #[test]
    fn a_key_no_host_shows_is_refused() {
        check!(Receiver::default().receive("render_sparkles", Value::from(true)).is_err());
        check!(Receiver::default().receive("fps", Value::from("fast")).is_err());
    }

    #[test]
    fn keys_suit_both_hosts() {
        for prop in props(Host::Lively) {
            let k = &prop.key;
            check!(k.starts_with(|c: char| c.is_ascii_lowercase()), "{k}");
            check!(k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{k}");
        }
    }
}
