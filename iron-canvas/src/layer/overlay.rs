use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use crate::layer::{create_2d_context, LayerBase};
use crate::renderer::{LayerOps, OverlayRenderer};
use crate::types::coord::{AutofillTarget, SheetArea};
use crate::{CanvasModel, FormulaRef, RCRange};

use crate::geometry::frame::FrameContext;

/// Overlay ranges passed to `render()`.
///
/// Selection is not stored here — it is paint-time-derived from
/// `model.get_selected_view()`. The consumer signals selection changes via
/// `IronCanvas::request_overlay_repaint()`.
#[derive(Clone, PartialEq, Default)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
}

pub(crate) struct OverlayLayer {
    base: LayerBase<OverlayRenderer>,
}

impl OverlayLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, true, true)?;
        let renderer = OverlayRenderer::for_layer(ctx);
        Ok(Self {
            base: LayerBase::new(canvas, renderer),
        })
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.base.mark_dirty();
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        self.base.should_paint()
    }

    pub(crate) fn paint(
        &mut self,
        overlays: &RenderOverlays,
        model: &dyn CanvasModel,
        frame: &FrameContext,
    ) {
        let size = frame.canvas_size;
        self.base
            .renderer
            .ctx_ref()
            .clear_rect(0.0, 0.0, size.w, size.h);
        self.base.renderer.render_overlays(model, overlays, frame);
    }

    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        self.base.resize(css_w, css_h, dpr);
    }
}
