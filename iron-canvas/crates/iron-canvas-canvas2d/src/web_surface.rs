//! `WebSurface` — the `Surface` adapter for HTML `<canvas>` + Canvas-2D.
//!
//! One `WebSurface` per `<canvas>` element: `IronCanvas` builds two, one
//! `grid` and one `overlay` (`alpha: true`, overlay also
//! `desynchronized: true`). Both contexts keep their alpha channel, so an
//! empty or resized canvas is transparent instead of opaque black.
//! Sheet backgrounds must still be opaque: retained paint uses background
//! fills to erase old content. The painter is
//! wrapped in `Rc` so the renderer can hold its own owning handle to the
//! same painter without ever lifetime-borrowing through the surface. Grid is
//! double-buffered (paints into a detached back canvas, `present` flips it to
//! the visible front canvas under the `copy` composite op); overlay draws
//! direct.

use std::rc::Rc;

use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, js_sys};

use iron_canvas_core::geometry::CanvasMetrics;
use iron_canvas_core::layer::Surface;

use crate::canvas_painter::CanvasPainter;

/// Composite op for the grid's back-to-front present copy. Replaces the
/// destination region instead of blending over it, so transparent back-buffer
/// pixels clear the front canvas rather than leaving stale pixels behind.
const PRESENT_COMPOSITE: &str = "copy";

pub struct WebSurface {
    canvas: HtmlCanvasElement,
    painter: Rc<CanvasPainter>,
    /// Double-buffer state — `Some` only for the grid surface. `back` is the
    /// detached canvas the painter draws into; `front_ctx` is the visible
    /// canvas' own 2d context, used only by `present`.
    back: Option<HtmlCanvasElement>,
    front_ctx: Option<CanvasRenderingContext2d>,
}

impl WebSurface {
    pub fn grid(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let back = create_detached_canvas(&canvas)?;
        let back_ctx = create_2d_context(&back, true, false)?;
        let front_ctx = create_2d_context(&canvas, true, false)?;
        // The 1:1 present copy must never resample; re-pinned on resize too
        // because a backing-store resize wipes ctx state.
        front_ctx.set_image_smoothing_enabled(false);
        // Replace front pixels, including regions cleared in the back buffer.
        // Resize resets context state, so it must restore this operation.
        front_ctx.set_global_composite_operation(PRESENT_COMPOSITE)?;
        Ok(Self {
            painter: Rc::new(CanvasPainter::with_blit_source(back_ctx, canvas.clone())),
            canvas,
            back: Some(back),
            front_ctx: Some(front_ctx),
        })
    }

    pub fn overlay(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, true, true)?;
        Ok(Self {
            painter: Rc::new(CanvasPainter::new(ctx)),
            canvas,
            back: None,
            front_ctx: None,
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

    fn resize(&mut self, metrics: CanvasMetrics) {
        let (target_w, target_h) = metrics.backing_size();
        for c in std::iter::once(&self.canvas).chain(self.back.iter()) {
            if c.width() != target_w || c.height() != target_h {
                c.set_width(target_w);
                c.set_height(target_h);
            }
        }
        if let Some(front) = &self.front_ctx {
            front.set_image_smoothing_enabled(false);
            // A backing-store resize resets the ctx drawing state, so both
            // pins made in `grid` must be re-applied here.
            let _ = front.set_global_composite_operation(PRESENT_COMPOSITE);
        }
        // `LayerBase::resize` follows up with `LayerOps::resize_for_dpr`,
        // which routes through `RendererCore::resize_for_dpr` and calls
        // `apply_dpr_transform` + `invalidate_cache` on the painter we share.
    }

    fn present(&self) {
        // Overlay surfaces draw direct — nothing to flip.
        let (Some(back), Some(front)) = (&self.back, &self.front_ctx) else {
            return;
        };
        // front_ctx carries no transform, so this is a 1:1 backing-pixel copy.
        // `PRESENT_COMPOSITE` (`copy`, pinned at construction and re-pinned
        // on resize) makes it a replacement, not a blend — see that constant.
        let _ = front.draw_image_with_html_canvas_element(back, 0.0, 0.0);
    }
}

/// Back buffer for the grid layer. A detached `<canvas>` element (not an
/// `OffscreenCanvas`): it keeps the ctx type `CanvasRenderingContext2d`, so
/// `CanvasPainter` needs no second context plumbing.
fn create_detached_canvas(sibling: &HtmlCanvasElement) -> Result<HtmlCanvasElement, JsValue> {
    sibling
        .owner_document()
        .ok_or_else(|| JsValue::from_str("grid canvas has no owner document"))?
        .create_element("canvas")?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| JsValue::from_str("created element is not a canvas"))
}

/// Build the 2D context with the given options. Both surfaces keep alpha.
/// The overlay also uses `desynchronized: true` for direct updates.
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
