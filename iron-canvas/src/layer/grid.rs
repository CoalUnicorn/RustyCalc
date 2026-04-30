use wasm_bindgen::{JsCast, JsValue};
use web_sys::{js_sys, CanvasRenderingContext2d, HtmlCanvasElement};

use crate::geometry::FrameContext;
use crate::layer::LayerBase;
use crate::theme::CanvasTheme;
use crate::theme::LIGHT;
use crate::CanvasModel;
use crate::CanvasRenderer;
use crate::CanvasSize;

pub(crate) struct GridLayer {
    base: LayerBase,
    // gate: PaintGate,
    // Long-lived renderer; owns the layer's 2D ctx so paint caches persist
    // across frames. Theme is hot-swapped per paint via `set_theme`.
    //renderer: CanvasRenderer,
}

impl GridLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let ctx_opts = js_sys::Object::new();
        js_sys::Reflect::set(&ctx_opts, &"alpha".into(), &JsValue::from(false))
            .map_err(|_| JsValue::from_str("failed to set grid context alpha option"))?;
        let ctx = canvas
            .get_context_with_context_options("2d", &ctx_opts)?
            .ok_or_else(|| JsValue::from_str("grid canvas 2d context unavailable"))?
            .unchecked_into::<CanvasRenderingContext2d>();
        let renderer = CanvasRenderer::for_layer(ctx, 0.0, 0.0, LIGHT);
        Ok(Self {
            //canvas: canvas.clone(),
            //gate: PaintGate::new(),
            //renderer: renderer.clone(),
            base: LayerBase::new(canvas, renderer),
        })
    }

    pub(crate) fn paint(
        &mut self,
        theme: CanvasTheme,
        model: &dyn CanvasModel,
        frame: &FrameContext, // pre-built by orchestrator
    ) {
        self.base.renderer.set_theme(theme);
        let size = self.base.canvas_size();
        let ctx = self.base.renderer.ctx_ref();
        ctx.set_fill_style_str(theme.cell_bg);
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

    pub(crate) fn canvas_size(&self) -> CanvasSize {
        self.base.canvas_size()
    }

    /// Resize the backing store. Ports the guard from `CanvasRenderer::new`:
    /// only reallocates the bitmap when dimensions actually change.
    pub(crate) fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        self.base.resize(css_w, css_h, dpr);
    }
}
