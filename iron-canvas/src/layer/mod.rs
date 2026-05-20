//! Two-canvas layering glue.
//!
//! `LayerBase` wraps an `HtmlCanvasElement` with a `PaintGate` (typed
//! `GridSignals` dirty bits) and a layer-specific renderer. `GridLayer`
//! and `OverlayLayer` instantiate the pair; the overlay decoration impls
//! live in the `decoration/` submodule.

mod grid;
mod overlay;

pub(crate) use grid::GridLayer;
pub(crate) use iron_canvas_core::decoration::{
    autofill::AutofillLayer, clipboard::ClipboardLayer, formula_refs::FormulaRefsLayer,
    point_mode::PointModeLayer, selection::SelectionLayer, Layer,
};
pub(crate) use overlay::OverlayLayer;
pub use overlay::RenderOverlays;
use std::cell::Cell;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::renderer::LayerOps;
use crate::signal::GridSignals;
use crate::CanvasSize;

pub(crate) struct PaintGate {
    signals: Cell<GridSignals>,
    #[cfg(test)]
    pub(crate) paint_count: Cell<u32>,
}

pub(crate) struct LayerBase<R: LayerOps> {
    pub(crate) canvas: HtmlCanvasElement,
    gate: PaintGate,
    pub(crate) renderer: R,
}

/// Build the 2D context with the given options. Both layers want the same
/// `js_sys::Object` + `Reflect::set` dance; only the booleans differ. Grid
/// uses `alpha: false` for opaque compositing; overlay uses
/// `alpha: true, desynchronized: true` so transparent updates can land
/// without a full present. Free fn rather than a method on `LayerBase<R>`
/// because R can't be inferred at the create-time call site.
pub(crate) fn create_2d_context(
    canvas: &HtmlCanvasElement,
    alpha: bool,
    desynchronized: bool,
) -> Result<CanvasRenderingContext2d, JsValue> {
    let ctx_opts = js_sys::Object::new();
    js_sys::Reflect::set(&ctx_opts, &"alpha".into(), &JsValue::from(alpha))
        .map_err(|_| JsValue::from_str("failed to set canvas context alpha option"))?;
    if desynchronized {
        js_sys::Reflect::set(&ctx_opts, &"desynchronized".into(), &JsValue::from(true))
            .map_err(|_| JsValue::from_str("failed to set canvas context desynchronized option"))?;
    }
    canvas
        .get_context_with_context_options("2d", &ctx_opts)?
        .ok_or_else(|| JsValue::from_str("canvas 2d context unavailable"))
        .map(|c| c.unchecked_into::<CanvasRenderingContext2d>())
}

impl<R: LayerOps> LayerBase<R> {
    pub(crate) fn new(canvas: HtmlCanvasElement, renderer: R) -> Self {
        Self {
            canvas,
            gate: PaintGate::new(),
            renderer,
        }
    }

    /// Back-compat shim: callers that don't yet know which signal they
    /// raise get the safest blanket. Stage 3 narrows this per setter.
    pub(crate) fn mark_dirty(&self) {
        self.gate
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
    }

    pub(crate) fn raise(&self, sig: GridSignals) {
        self.gate.raise(sig);
    }

    pub(crate) fn drain_signals(&self) -> GridSignals {
        self.gate.drain()
    }

    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        let (target_w, target_h) = CanvasSize {
            w: f64::from(css_w),
            h: f64::from(css_h),
        }
        .to_backing_size(dpr);
        if self.canvas.width() != target_w || self.canvas.height() != target_h {
            self.canvas.set_width(target_w);
            self.canvas.set_height(target_h);
        }
        // resize_for_dpr unconditionally resets the transform before scaling,
        // so the prior `else { set_transform(identity) }` branch is no longer
        // needed — the path taken when only DPR changed is now the same as
        // the path taken after a backing-store reallocation.
        self.renderer.resize_for_dpr(dpr);
    }
}

impl PaintGate {
    pub(crate) fn new() -> Self {
        Self {
            signals: Cell::new(GridSignals::EMPTY),
            #[cfg(test)]
            paint_count: Cell::new(0),
        }
    }

    pub(crate) fn raise(&self, sig: GridSignals) {
        self.signals.set(self.signals.get() | sig);
    }

    pub(crate) fn drain(&self) -> GridSignals {
        let drained = self.signals.replace(GridSignals::EMPTY);
        #[cfg(test)]
        if !drained.is_empty() {
            self.paint_count.set(self.paint_count.get() + 1);
        }
        drained
    }

    // Back-compat shims for tests. Production code goes through `LayerBase`.
    #[cfg(test)]
    pub(crate) fn mark_dirty(&self) {
        self.raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
    }

    #[cfg(test)]
    pub(crate) fn should_paint(&self) -> bool {
        !self.drain().is_empty()
    }
}
