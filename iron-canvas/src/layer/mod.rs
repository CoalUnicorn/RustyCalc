mod grid;
mod overlay;

pub(crate) use grid::GridLayer;
pub(crate) use overlay::OverlayLayer;
pub use overlay::RenderOverlays;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::renderer::LayerOps;
use crate::CanvasSize;

pub(crate) struct PaintGate {
    dirty: bool,
    #[cfg(test)]
    pub(crate) paint_count: u32,
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

    pub(crate) fn mark_dirty(&mut self) {
        self.gate.mark_dirty();
    }

    /// Consume the dirty flag. Returns `true` if a paint is needed.
    pub(crate) fn should_paint(&mut self) -> bool {
        self.gate.should_paint()
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
        } else {
            self.renderer
                .ctx_ref()
                .set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .expect("set_transform should not fail");
        }
        self.renderer
            .ctx_ref()
            .scale(f64::from(dpr), f64::from(dpr))
            .expect("scale should not fail");
        self.renderer.set_dpr(dpr);
        self.renderer.invalidate_paint_cache();
    }
}

impl PaintGate {
    pub(crate) fn new() -> Self {
        Self {
            dirty: false,
            #[cfg(test)]
            paint_count: 0,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        let was_dirty = std::mem::replace(&mut self.dirty, false);
        #[cfg(test)]
        if was_dirty {
            self.paint_count += 1;
        }
        was_dirty
    }
}
