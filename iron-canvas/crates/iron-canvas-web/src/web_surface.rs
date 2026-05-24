//! `WebSurface` — the `Surface` adapter for HTML `<canvas>` + Canvas-2D.
//!
//! One `WebSurface` per `<canvas>` element: `IronCanvas` builds two, one
//! `grid` (opaque) and one `overlay` (`alpha: true, desynchronized: true`).
//! The painter is wrapped in `Rc` so the renderer can hold its own owning
//! handle to the same painter without ever lifetime-borrowing through the
//! surface.

use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;

use crate::canvas_painter::CanvasPainter;

pub struct WebSurface {
    canvas: HtmlCanvasElement,
    painter: Rc<CanvasPainter>,
}

impl WebSurface {
    pub fn grid(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, false, false)?;
        Ok(Self {
            canvas,
            painter: Rc::new(CanvasPainter::new(ctx)),
        })
    }

    pub fn overlay(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, true, true)?;
        Ok(Self {
            canvas,
            painter: Rc::new(CanvasPainter::new(ctx)),
        })
    }

    /// Underlying `<canvas>` element. Exposed so the orchestrator can
    /// override inline CSS dimensions during playback (the CSS class
    /// pins display size to `100%`; without an inline override the
    /// backing-store resize would not move the display size).
    pub fn canvas(&self) -> &HtmlCanvasElement {
        &self.canvas
    }
}

impl Surface for WebSurface {
    type P = CanvasPainter;

    fn painter(&self) -> &CanvasPainter {
        self.painter.as_ref()
    }

    fn clone_painter(&self) -> Rc<CanvasPainter> {
        Rc::clone(&self.painter)
    }

    fn resize(&mut self, css: CanvasSize, dpr: i32) {
        let (target_w, target_h) = css.to_backing_size(dpr);
        if self.canvas.width() != target_w || self.canvas.height() != target_h {
            self.canvas.set_width(target_w);
            self.canvas.set_height(target_h);
        }
        // `LayerBase::resize` follows up with `LayerOps::resize_for_dpr`,
        // which routes through `RendererCore::resize_for_dpr` and calls
        // `apply_dpr_transform` + `invalidate_cache` on the painter we share.
        let _ = dpr;
    }

    fn present(&self) {
        // Canvas-2D auto-presents per draw call.
    }
}

/// Build the 2D context with the given options. Grid uses `alpha: false`
/// for opaque compositing; overlay uses `alpha: true, desynchronized: true`
/// so transparent updates can land without a full present.
fn create_2d_context(
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
