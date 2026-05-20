use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::{LayerBase, Surface};

use crate::canvas_painter::CanvasPainter;
use crate::chrome::Chrome;
use crate::layer::{Layer, SelectionLayer};
use crate::painter::Painter;
use crate::renderer::OverlayRenderer;
use crate::web_surface::WebSurface;
use crate::CanvasModel;
pub use iron_canvas_core::RenderOverlays;

pub(crate) struct OverlayLayer {
    base: LayerBase<WebSurface, OverlayRenderer<CanvasPainter>>,
}

impl OverlayLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let surface = WebSurface::overlay(canvas)?;
        let renderer = OverlayRenderer::for_layer(surface.clone_painter());
        Ok(Self {
            base: LayerBase::new(surface, renderer),
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
            .surface
            .painter()
            .ctx()
            .clear_rect(0.0, 0.0, size.w, size.h);

        let painter = self.base.surface.painter();
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
        selection.paint_after_hook(model, frame, self.base.surface.painter());

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

        let painter = self.base.surface.painter();
        for layer in others {
            layer.paint(model, frame, painter);
        }
        painter.end_group();
    }

    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        self.base.resize(
            CanvasSize {
                w: f64::from(css_w),
                h: f64::from(css_h),
            },
            dpr,
        );
    }
}
