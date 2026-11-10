//! Browser entry point: reads the page URL's settings and runs the viewer on a canvas.

#![cfg(target_arch = "wasm32")]

mod settings;

use std::cell::RefCell;

use currents_core::viewer::{self, Hooks};
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
    if console_log::init_with_level(log::Level::Info).is_err() {
        log::warn!("logger already installed");
    }
}

/// Starts the viewer on `canvas`. `on_ready()` runs after the first frame and `on_error(message)`
/// on any failure that stops it.
#[wasm_bindgen]
pub fn start(canvas: HtmlCanvasElement, on_ready: Function, on_error: Function) -> Result<(), JsError> {
    ON_ERROR.with(|h| *h.borrow_mut() = Some(on_error));
    let hooks = Hooks {
        on_ready: Some(Box::new(move || {
            let _ = on_ready.call0(&JsValue::NULL);
        })),
        on_error: Some(Box::new(|message| {
            log::error!("{message}");
            report(&message);
        })),
    };
    viewer::spawn(settings::from_url(), canvas, hooks).map_err(|e| JsError::new(&e.to_string()))
}
