//! Browser entry point: reads the page URL's settings and runs the viewer on a canvas.

#![cfg(target_arch = "wasm32")]

mod settings;

use std::cell::{Cell, RefCell};

use currents_core::hosts::Receiver;
use currents_core::settings::toml::Value;
use currents_core::viewer::{self, Conditions, Failure, GpuReport, Hooks, Remote};
use js_sys::{Function, Object, Reflect};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

thread_local! {
    static ON_ERROR: RefCell<Option<Function>> = const { RefCell::new(None) };
    /// Whether the viewer has told the page it is up, which a crash report carries.
    static READY: Cell<bool> = const { Cell::new(false) };
}

/// Hands a failure to the page's error handler, if one was registered.
fn report(stage: &str, message: &str, detail: &str, drawn: bool) {
    let handler = ON_ERROR.with(|h| h.borrow().clone());
    let Some(handler) = handler else { return };
    let failure = object(&[
        ("stage", JsValue::from_str(stage)),
        ("message", JsValue::from_str(message)),
        ("detail", JsValue::from_str(detail)),
        ("drawn", JsValue::from_bool(drawn)),
    ]);
    if let Err(error) = handler.call1(&JsValue::NULL, &failure) {
        log::warn!("the page's error handler threw: {error:?}");
    }
}

fn object(fields: &[(&str, JsValue)]) -> Object {
    let object = Object::new();
    for (key, value) in fields {
        let _ = Reflect::set(&object, &JsValue::from_str(key), value);
    }
    object
}

fn gpu_object(report: &GpuReport) -> Object {
    object(&[
        ("adapter", JsValue::from_str(&report.adapter)),
        ("backend", JsValue::from_str(&report.backend)),
        ("driver", JsValue::from_str(&report.driver)),
        ("float32Filterable", JsValue::from_bool(report.float32_filterable)),
        ("timestamps", JsValue::from_bool(report.timestamps)),
        ("maxTextureSide", JsValue::from(report.max_texture_side)),
        ("surfaceFormat", JsValue::from_str(&report.surface_format)),
    ])
}

#[wasm_bindgen(start)]
fn init() {
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        report("crash", &format!("the renderer crashed: {what}"), &info.to_string(), READY.get());
    }));
    if console_log::init_with_level(settings::log_level()).is_err() {
        log::warn!("logger already installed");
    }
}

/// A running viewer, for the page and wallpaper hosts to change while it runs.
#[wasm_bindgen]
pub struct Viewer {
    remote: Remote,
    host: RefCell<Receiver>,
    conditions: Cell<Conditions>,
}

#[wasm_bindgen]
impl Viewer {
    /// Changes the setting at dotted `path`, as in `motion.speed`, to a number, boolean or key
    /// chord; `null` switches an optional setting off. A value the setting cannot take is
    /// logged and changes nothing.
    pub fn set(&self, path: &str, value: &JsValue) -> Result<(), JsError> {
        self.remote.set(path, value_of(value).map_err(|e| JsError::new(&format!("{path}: {e}")))?);
        Ok(())
    }

    /// Stops drawing while `held`, as a wallpaper host asks while nothing shows the page.
    pub fn hold(&self, held: bool) {
        self.remote.hold(held);
    }

    /// Whether the device runs on battery, for the battery settings; the page asks the browser.
    pub fn power(&self, on_battery: bool) {
        let conditions = Conditions { on_battery, ..self.conditions.get() };
        self.conditions.set(conditions);
        self.remote.conditions(conditions);
    }

    /// Ends the viewer and releases its graphics device, before the page removes its canvas.
    pub fn stop(&self) {
        self.remote.stop();
    }

    /// Takes a property from a wallpaper host's panel, under the key the host package gave it.
    pub fn host(&self, key: &str, value: &JsValue) -> Result<(), JsError> {
        let value = value_of(value).map_err(|e| JsError::new(&format!("{key}: {e}")))?;
        let value = value.ok_or_else(|| JsError::new(&format!("{key}: no value")))?;
        for (path, value) in self.host.borrow_mut().receive(key, value).map_err(|e| JsError::new(&e))? {
            self.remote.set(path, value);
        }
        Ok(())
    }
}

/// A page value as a settings value: a whole number stays whole so frame rates can take it.
fn value_of(value: &JsValue) -> Result<Option<Value>, String> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    if let Some(b) = value.as_bool() {
        return Ok(Some(Value::from(b)));
    }
    if let Some(n) = value.as_f64() {
        let whole = n.fract() == 0.0 && n.abs() < 9.0e15;
        #[expect(clippy::cast_possible_truncation, reason = "whole and well inside i64")]
        return Ok(Some(if whole { Value::from(n as i64) } else { Value::from(n) }));
    }
    value.as_string().map(|s| Some(Value::from(s))).ok_or_else(|| "expected a number, boolean or text".to_owned())
}

/// Puts a link to `settings` in the address bar and on the clipboard. The link names only
/// what differs from a page with nothing in its URL.
fn share(settings: &currents_core::settings::Settings) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let params = web_sys::UrlSearchParams::new()?;
    for (path, value) in currents_core::settings::toml::to_pairs(&settings::base(), settings) {
        params.append(&path, &value);
    }
    let location = window.location();
    let query = String::from(params.to_string());
    let mut url = format!("{}{}", location.origin()?, location.pathname()?);
    if !query.is_empty() {
        url = format!("{url}?{query}");
    }
    window.history()?.replace_state_with_url(&JsValue::NULL, "", Some(&url))?;
    // The clipboard answers later; a refusal still leaves the link in the address bar.
    let _ = window.navigator().clipboard().write_text(&url);
    Ok(())
}

/// A function the page passed under `key` in the start options, if it passed one.
fn callback(options: &JsValue, key: &str) -> Option<Function> {
    Reflect::get(options, &JsValue::from_str(key)).ok()?.dyn_into().ok()
}

fn call(function: &Function, argument: &JsValue) {
    if let Err(error) = function.call1(&JsValue::NULL, argument) {
        log::warn!("a page callback threw: {error:?}");
    }
}

/// Starts the viewer on `canvas`. `options` carries the page's callbacks and choices:
///
/// - `onReady()` once the first frame is done on the device;
/// - `onError({stage, message, drawn})` when the viewer stops, or a running one loses its device;
/// - `onGpu({adapter, backend, ...})` once the device is open;
/// - `notice`, text shown as a toast once the viewer is up;
/// - `share`, true to offer the menu's Copy link button;
/// - `strict`, true to fail over a fault before the first frame instead of drawing black.
#[wasm_bindgen]
pub fn start(canvas: HtmlCanvasElement, options: &JsValue) -> Result<Viewer, JsError> {
    let flag = |key: &str| Reflect::get(options, &JsValue::from_str(key)).is_ok_and(|v| v.is_truthy());
    ON_ERROR.with(|h| *h.borrow_mut() = callback(options, "onError"));
    READY.set(false);
    let on_ready = callback(options, "onReady");
    let on_gpu = callback(options, "onGpu");
    let hooks = Hooks {
        on_ready: Some(Box::new(move || {
            READY.set(true);
            if let Some(on_ready) = on_ready {
                call(&on_ready, &JsValue::UNDEFINED);
            }
        })),
        on_error: Some(Box::new(|Failure { stage, message, detail, drawn }| {
            log::error!("{detail}");
            report(stage.into(), &message, &detail, drawn);
        })),
        on_gpu: on_gpu.map(|on_gpu| -> viewer::OnGpu { Box::new(move |gpu| call(&on_gpu, &gpu_object(gpu))) }),
        share: flag("share").then(|| -> viewer::Share {
            Box::new(|settings| {
                if let Err(error) = share(settings) {
                    log::warn!("could not share the settings: {error:?}");
                }
            })
        }),
        notice: Reflect::get(options, &JsValue::from_str("notice")).ok().and_then(|v| v.as_string()),
        strict_start: flag("strict"),
    };
    viewer::spawn(settings::from_url(), canvas, hooks)
        .map(|remote| Viewer { remote, host: RefCell::default(), conditions: Cell::default() })
        .map_err(|e| JsError::new(&e.to_string()))
}
