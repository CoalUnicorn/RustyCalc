use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use crate::chrome::{BlitPlan, Chrome};
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
        frame: &Chrome, // pre-built by orchestrator
    ) {
        // Full-canvas clear runs only on rebuild. On `slots_reused` the
        // prior frame's pixels stay; `render_pane` clears its own pane
        // bg before re-painting when its fingerprint changed, preserving
        // the fingerprint-skip win for clean panes.
        if !frame.slots_reused {
            let size = frame.canvas_size;
            let ctx = self.base.renderer.ctx_ref();
            ctx.set_fill_style_str(frame.theme.cell_bg.as_ref());
            ctx.fill_rect(0.0, 0.0, size.w, size.h);
        }
        self.base.renderer.invalidate_paint_cache();
        self.base.renderer.render_grid(model, frame);
    }

    /// Scroll-blit fast path: shift the BottomRight kept band via
    /// `Painter::blit`, then re-run the grid pipeline with the BottomRight
    /// pane wrapped in a clip to `plan.repaint_strip`. The caller (the
    /// orchestrator) must set `frame.slots_reused = true` so render_pane's
    /// fingerprint-mismatch branch fills the pane bg — clipped to the
    /// strip, this erases stale pixels there without touching the kept
    /// band. Falls back to `paint` if the backend doesn't support `blit`.
    pub(crate) fn painter_supports_blit(&self) -> bool {
        self.base.renderer.painter_supports_blit()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn bottom_right_cache_range_debug(&self) -> Option<crate::RCRange> {
        self.base
            .renderer
            .pane_cache_range_debug(crate::chrome::PaneRegion::BottomRight)
    }

    pub(crate) fn paint_blit(&mut self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        if !self.base.renderer.painter_supports_blit() {
            self.paint(model, frame);
            return;
        }
        self.base.renderer.painter_blit(plan.src, plan.dst);
        // `drawImage` doesn't disturb ctx fillStyle / strokeStyle / font /
        // lineWidth, so the painter's state cache is still valid across the
        // blit. Letting it survive lets the subsequent strip paint reuse
        // the prior frame's setter binds instead of re-issuing them.
        //
        // `render_grid_blit` rotates the cached pane buffers at its top
        // (`prepare_blit_cache`) before any cell paint — the blit must
        // come first so the kept pixels are in their new position when
        // the strip-fetch paints over the revealed band.
        self.base.renderer.render_grid_blit(model, frame, plan);
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
