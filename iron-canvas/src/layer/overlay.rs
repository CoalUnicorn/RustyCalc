use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use crate::chrome::Chrome;
use crate::layer::{create_2d_context, Layer, LayerBase, SelectionLayer};
use crate::painter::{CanvasPainter, Painter};
use crate::renderer::OverlayRenderer;
use crate::types::coord::{AutofillTarget, SheetArea};
use crate::{CanvasModel, FormulaRef, RCRange};

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
    base: LayerBase<OverlayRenderer<CanvasPainter>>,
}

impl OverlayLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, true, true)?;
        let renderer = OverlayRenderer::for_layer(ctx);
        Ok(Self {
            base: LayerBase::new(canvas, renderer),
        })
    }

    #[allow(dead_code)] // Back-compat shim; production callers use `raise`.
    pub(crate) fn mark_dirty(&self) {
        self.base.mark_dirty();
    }

    pub(crate) fn raise(&self, sig: crate::signal::GridSignals) {
        self.base.raise(sig);
    }

    pub(crate) fn drain_signals(&self) -> crate::signal::GridSignals {
        self.base.drain_signals()
    }

    pub(crate) fn paint(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        selection: &SelectionLayer,
        others: &[&dyn Layer],
    ) {
        let size = frame.canvas_size;
        self.base
            .renderer
            .ctx_ref()
            .clear_rect(0.0, 0.0, size.w, size.h);

        let painter = self.base.renderer.painter();
        painter.begin_group("overlay");

        // Selection paints fill (under) then stroke + handle (over) the
        // active-cell repaint. Header highlights land between selection
        // and the rest so the highlighted header strip is above the
        // selection tint.
        selection.paint(model, frame, painter);
        if let Some(hook) = selection.after_paint_renderer_hook(model, frame) {
            self.base
                .renderer
                .repaint_active_cell(model, hook.row, hook.col, frame);
        }
        selection.paint_after_hook(model, frame, self.base.renderer.painter());

        self.base.renderer.render_header_highlights(
            crate::geometry::prim::Axis::Row,
            frame,
            selection.selection_range,
        );
        self.base.renderer.render_header_highlights(
            crate::geometry::prim::Axis::Column,
            frame,
            selection.selection_range,
        );

        let painter = self.base.renderer.painter();
        for layer in others {
            layer.paint(model, frame, painter);
        }
        painter.end_group();
    }

    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        self.base.resize(css_w, css_h, dpr);
    }
}
