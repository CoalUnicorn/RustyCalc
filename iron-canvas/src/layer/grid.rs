use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use crate::geometry::frame::FrameContext;
use crate::layer::{create_2d_context, LayerBase};
use crate::painter::CanvasPainter;
use crate::renderer::GridRenderer;
use crate::CanvasModel;

pub(crate) struct GridLayer {
    base: LayerBase<GridRenderer<CanvasPainter>>,
}

impl GridLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx = create_2d_context(&canvas, false, false)?;
        let renderer = GridRenderer::for_layer(ctx);
        Ok(Self {
            base: LayerBase::new(canvas, renderer),
        })
    }

    pub(crate) fn paint(
        &mut self,
        model: &dyn CanvasModel,
        frame: &FrameContext, // pre-built by orchestrator
    ) {
        let size = frame.canvas_size;
        let ctx = self.base.renderer.ctx_ref();
        ctx.set_fill_style_str(frame.theme.cell_bg.as_ref());
        ctx.fill_rect(0.0, 0.0, size.w, size.h);
        self.base.renderer.invalidate_paint_cache();
        self.base.renderer.render_grid(model, frame);
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.base.mark_dirty();
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        self.base.should_paint()
    }

    /// Resize the backing store. Ports the guard from `CanvasRenderer::new`:
    /// only reallocates the bitmap when dimensions actually change.
    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        self.base.resize(css_w, css_h, dpr);
    }
}
