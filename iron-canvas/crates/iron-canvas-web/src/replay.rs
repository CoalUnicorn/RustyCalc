//! `.icr` replay entry points for the dev recording viewer.
//!
//! The viewer (web-test/recording-viewer.html) drives a *cumulative*
//! grid canvas and a *per-frame* overlay canvas. Both consume the same
//! `DrawOp` slices the renderer emits — the split is purely so the JS
//! side can decide cadence (anchor at last `Fresh`, paint forward for
//! grid; clear + paint single frame for overlay).
//!
//! Both wasm fns wrap `iron_canvas_recorder::replay`, so the dispatch
//! is the same trait-call sequence the live renderer uses against
//! `CanvasPainter`. No JS-side switch.

use iron_canvas_recorder::{DrawOp, replay};
use wasm_bindgen::prelude::*;
use web_sys::CanvasRenderingContext2d;

use crate::CanvasPainter;

#[wasm_bindgen(js_name = icrReplayGridOps)]
pub fn icr_replay_grid_ops(ctx: CanvasRenderingContext2d, ops_json: &str) -> Result<(), JsValue> {
    replay_into(ctx, ops_json)
}

#[wasm_bindgen(js_name = icrReplayOverlayOps)]
pub fn icr_replay_overlay_ops(
    ctx: CanvasRenderingContext2d,
    ops_json: &str,
) -> Result<(), JsValue> {
    replay_into(ctx, ops_json)
}

fn replay_into(ctx: CanvasRenderingContext2d, ops_json: &str) -> Result<(), JsValue> {
    let ops: Vec<DrawOp> = serde_json::from_str(ops_json)
        .map_err(|e| JsValue::from_str(&format!("icr ops parse: {e}")))?;
    let painter = CanvasPainter::new(ctx);
    replay(&painter, &ops);
    Ok(())
}
