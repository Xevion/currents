//! Browser entry point: reads the page URL's settings and runs the viewer on a canvas.

#![cfg(target_arch = "wasm32")]

mod settings;

use std::cell::{Cell, RefCell};

use currents_core::hosts::Receiver;
use currents_core::settings::toml::Value;
use currents_core::viewer::{self, Conditions, Hooks, Remote};
use js_sys::Function;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

thread_local! {
    static ON_ERROR: RefCell<Option<Function>> = const { RefCell::new(None) };
}

/// Passes a failure description to the page's error handler, if one was registered.
fn report(message: &str) {
    let handler = ON_ERROR.with(|h| h.borrow().clone());
    if let Some(handler) = handler {
        let _ = handler.call1(&JsValue::NULL, &JsValue::from_str(message));
    }
}

#[wasm_bindgen(start)]
fn init() {
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        report("The renderer crashed.");
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

/// Starts the viewer on `canvas`. `on_ready()` runs after the first frame and `on_error(message)`
/// on any failure that stops it.
#[wasm_bindgen]
pub fn start(canvas: HtmlCanvasElement, on_ready: Function, on_error: Function) -> Result<Viewer, JsError> {
    ON_ERROR.with(|h| *h.borrow_mut() = Some(on_error));
    let hooks = Hooks {
        on_ready: Some(Box::new(move || {
            let _ = on_ready.call0(&JsValue::NULL);
        })),
        on_error: Some(Box::new(|message| {
            log::error!("{message}");
            report(&message);
        })),
        share: Some(Box::new(|settings| {
            if let Err(error) = share(settings) {
                log::warn!("could not share the settings: {error:?}");
            }
        })),
    };
    viewer::spawn(settings::from_url(), canvas, hooks)
        .map(|remote| Viewer { remote, host: RefCell::default(), conditions: Cell::default() })
        .map_err(|e| JsError::new(&e.to_string()))
}
