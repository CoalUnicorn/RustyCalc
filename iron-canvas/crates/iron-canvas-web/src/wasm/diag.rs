//! Dep-free `console.warn` shim.
//!
//! Shared by this crate's two diagnostic surfaces: `JsBackedModel`'s
//! throw / serde counters and the recording watchdog in `orchestrator`
//! (soft-warn / hard-cap). The Canvas2D painter lives in the separate
//! `iron-canvas-canvas2d` crate and carries its own local `console.warn`
//! binding (its `measure_text_width` fallback), so it does not route here.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    pub(crate) fn console_warn(s: &str);
}
