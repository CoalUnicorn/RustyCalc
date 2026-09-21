//! Thin `console.warn` binding.
//!
//! Two callers share it: `JsBackedModel`'s throw / serde counters
//! (`js-model`) and the recording watchdog in `orchestrator` (soft-warn /
//! hard-cap, `dev-tools`). Neither feature owns it, so it lives at the crate
//! root. The Canvas2D painter lives in the separate `iron-canvas-canvas2d`
//! crate and carries its own local `console.warn` binding (its
//! `measure_text_width` fallback), so it does not route here.

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    pub(crate) fn warn(s: &str);
}
