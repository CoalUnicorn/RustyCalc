//! Dep-free `console.warn` shim.
//!
//! Lives outside `wasm.rs` so both the JS-bridge diagnostics layer
//! (`JsBackedModel` throw / serde counters) and `painter::canvas`
//! (`measure_text_width` fallback) can call into the same binding
//! without `painter -> wasm` becoming a layering smell.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    pub(crate) fn console_warn(s: &str);

    #[wasm_bindgen(js_namespace = console, js_name = log)]
    pub(crate) fn console_log(s: &str);
}
