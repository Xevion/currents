use currents_core::ParamOverride;
use currents_core::shaders::Sources;
use currents_core::viewer::{Budget, Settings};
use web_sys::UrlSearchParams;

const SPEED: f32 = 0.4;
/// The shade pass averages over each pixel itself, and a factor of 2 costs it four times as much.
const SUPERSAMPLING: u32 = 1;
/// Pixels shaded per frame before the render scale is applied; phones and laptops share
/// their GPU with the compositor, so this sits well under a large desktop window.
const MAX_PIXELS: f64 = 3_686_400.0;

/// Settings from the page URL: `speed`, `ss`, `aa` (`off` to stop blending frames), `field` (output pixels per flow field texel),
/// `p=INDEX=VALUE` (repeatable), `pixels` (the budget), `scale` (a fixed render scale, which
/// turns off adapting it and the field to the frame rate) and `debug` (a field view to start in).
pub(crate) fn from_url() -> Settings {
    let params =
        web_sys::window().and_then(|w| w.location().search().ok()).and_then(|s| UrlSearchParams::new_with_str(&s).ok());
    let get = |key: &str| params.as_ref().and_then(|p| p.get(key));
    let number = |key: &str| get(key).and_then(|s| s.trim().parse::<f64>().ok()).filter(|v| v.is_finite());

    let overrides: Vec<ParamOverride> = params
        .as_ref()
        .map(|p| p.get_all("p").iter().filter_map(|v| v.as_string()?.parse().ok()).collect())
        .unwrap_or_default();
    let scale = number("scale").filter(|&s| s > 0.0);
    Settings {
        sources: Sources::embedded(),
        speed: number("speed").map_or(SPEED, |v| v as f32),
        supersampling: number("ss").map_or(SUPERSAMPLING, |v| v.clamp(1.0, 4.0) as u32),
        temporal: get("aa").is_none_or(|s| s.trim() != "off"),
        field_step: number("field").map_or(1, |v| v.clamp(1.0, 4.0) as u32),
        params: ParamOverride::pack(&overrides),
        budget: Some(Budget {
            max_pixels: number("pixels").filter(|&v| v >= 1.0).unwrap_or(MAX_PIXELS),
            scale: scale.unwrap_or(1.0),
            adaptive: scale.is_none(),
        }),
        debug: get("debug").and_then(|s| s.trim().parse().ok()),
    }
}
