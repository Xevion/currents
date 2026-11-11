use currents_core::ParamOverride;
use currents_core::renderer::FieldView;
use currents_core::settings::{Settings, toml};
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

/// The settings a page starts from when its URL names none: a pixel budget the render scale
/// adapts to, since a browser shares its GPU with everything else on screen.
pub(crate) fn base() -> Settings {
    let mut settings = Settings::default();
    settings.render.max_pixels = Some(MAX_PIXELS);
    settings.render.adaptive = true;
    settings
}

/// Settings from the page URL: any setting by its dotted name as the Copy link button writes
/// them (`motion.speed=1.5`, `render.fps_cap=off`), and the short forms `speed`, `ss`, `aa`
/// (`off` to stop blending frames), `field` (output pixels per flow field texel),
/// `p=INDEX=VALUE` or `p=NAME=VALUE` (repeatable), `pixels` (the budget), `scale` (a fixed
/// render scale, which turns off adapting it and the field to the frame rate) and `debug` (a
/// field view to start in). Nothing is saved: changes made in the menu last until the page reloads.
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
    let mut settings = base();
    let r = &mut settings.render;
    if let Some(pixels) = number("pixels").filter(|&v| v >= 1.0) {
        r.max_pixels = Some(pixels);
    }
    if let Some(scale) = scale {
        r.render_scale = scale;
        r.adaptive = false;
    }
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
    let mut issues: Vec<String> = settings.clamp().iter().map(ToString::to_string).collect();
    let dotted: Vec<(String, String)> = params
        .as_ref()
        .and_then(|p| js_sys::try_iter(p).ok().flatten())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let pair = js_sys::Array::from(&entry.ok()?);
            Some((pair.get(0).as_string()?, pair.get(1).as_string()?))
        })
        .filter(|(key, _)| key.contains('.'))
        .collect();
    let (linked, link_issues) = toml::from_pairs(&settings, dotted.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    settings = linked;
    issues.extend(link_issues.iter().map(ToString::to_string));
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
