use currents_core::ParamOverride;
use currents_core::renderer::FieldView;
use currents_core::settings::Settings;
use currents_core::shaders::Sources;
use currents_core::viewer::{Launch, Placement};
use web_sys::UrlSearchParams;

/// The console's log level from the page URL's `log` (`debug`, `trace`, ...); `info` without one.
pub(crate) fn log_level() -> log::Level {
    let search = web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default();
    UrlSearchParams::new_with_str(&search)
        .ok()
        .and_then(|p| p.get("log"))
        .and_then(|level| level.parse().ok())
        .unwrap_or(log::Level::Info)
}

/// Pixels shaded per frame before the render scale is applied; phones and laptops share
/// their GPU with the compositor, so this sits well under a large desktop window.
const MAX_PIXELS: f32 = 3_686_400.0;

/// Settings from the page URL: `speed`, `ss`, `aa` (`off` to stop blending frames), `field` (output pixels per flow field texel),
/// `p=INDEX=VALUE` or `p=NAME=VALUE` (repeatable), `pixels` (the budget), `scale` (a fixed render scale, which
/// turns off adapting it and the field to the frame rate) and `debug` (a field view to start in).
/// Nothing is saved: changes made in the menu last until the page reloads.
pub(crate) fn from_url() -> Launch {
    let params =
        web_sys::window().and_then(|w| w.location().search().ok()).and_then(|s| UrlSearchParams::new_with_str(&s).ok());
    let get = |key: &str| params.as_ref().and_then(|p| p.get(key));
    let number = |key: &str| get(key).and_then(|s| s.trim().parse::<f32>().ok()).filter(|v| v.is_finite());

    let overrides: Vec<ParamOverride> = params
        .as_ref()
        .map(|p| p.get_all("p").iter().filter_map(|v| v.as_string()?.parse().ok()).collect())
        .unwrap_or_default();
    let scale = number("scale").filter(|&s| s > 0.0);
    let mut settings = Settings::default();
    let r = &mut settings.render;
    r.max_pixels = Some(number("pixels").filter(|&v| v >= 1.0).unwrap_or(MAX_PIXELS));
    r.render_scale = scale.unwrap_or(1.0);
    r.adaptive = scale.is_none();
    if let Some(ss) = number("ss") {
        r.supersampling = ss as u32;
    }
    if let Some(field) = number("field") {
        r.field_step = field as u32;
    }
    r.temporal = get("aa").is_none_or(|s| s.trim() != "off");
    if let Some(speed) = number("speed") {
        settings.motion.speed = speed;
    }
    ParamOverride::apply(&overrides, &mut settings.params.0);
    let issues = settings.clamp().iter().map(ToString::to_string).collect();
    Launch {
        settings,
        sources: Sources::embedded(),
        debug: get("debug").and_then(|s| s.trim().parse::<FieldView>().ok()),
        store: None,
        issues,
        placement: Placement::Window,
        on_window: None,
    }
}
