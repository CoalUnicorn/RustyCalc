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
        // Full-canvas clear runs only on Fresh frames. When the prior
        // frame's slot vecs were reused (SlotsReused), its pixels stay;
        // `render_pane` clears its own pane bg before re-painting when
        // its fingerprint changed, preserving the fingerprint-skip win
        // for clean panes. Blitted frames take `paint_blit`, which
        // never reaches this clear.
        //
        // Setter-cache invalidation is the orchestrator arm's
        // responsibility: `paint_rebuild` / `paint_content` call
        // `invalidate_paint_cache` at their prologue. The raw
        // `ctx.set_fill_style_str` below bypasses the cache but doesn't
        // read it, so an Empty cache here is safe — the first cached-set
        // inside `render_grid` will re-bind from Empty.
        if !frame.kind.reuses_slots() {
            let size = frame.canvas_size;
            let ctx = self.base.renderer.ctx_ref();
            ctx.set_fill_style_str(frame.theme.cell_bg.as_ref());
            ctx.fill_rect(0.0, 0.0, size.w, size.h);
        }
        self.base.renderer.render_grid(model, frame);
    }

    /// Wipe the painter's sticky `set_*_cached` state and reset text
    /// defaults. Paint regime arms (`paint_rebuild`, `paint_content`) call
    /// this at their prologue; arms that preserve ctx state
    /// (`paint_overlay`, `paint_viewport`) skip it.
    pub(crate) fn invalidate_paint_cache(&mut self) {
        self.base.renderer.invalidate_paint_cache();
    }

    pub(crate) fn invalidate_pane_cache(&self, mask: crate::chrome::PaneRegionMask) {
        self.base.renderer.invalidate_pane_cache(mask);
    }

    /// Scroll-blit fast path: shift the BottomRight kept band via
    /// `BlitPainter::blit`, then run `render_grid_blit` with the
    /// BottomRight pane wrapped in a clip to `plan.repaint_strip`. The
    /// orchestrator hands in a frame whose `kind` is `Blitted` so
    /// `render_pane_blit`'s strip-fetch branch fills only the revealed
    /// band — kept-band pixels remain untouched.
    pub(crate) fn paint_blit(&mut self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        for s in &plan.shifts {
            self.base.renderer.painter_blit(s.src, s.dst);
        }
        // `drawImage` doesn't disturb ctx fillStyle / strokeStyle / font /
        // lineWidth, so the painter's state cache is still valid across the
        // blit. Letting it survive lets the subsequent strip paint reuse
        // the prior frame's setter binds instead of re-issuing them.
        //
        // `render_grid_blit` rotates the cached pane buffers at its top —
        // the blit must come first so the kept pixels are in their new
        // position when the strip-fetch paints over the revealed band.
        self.base.renderer.render_grid_blit(model, frame, plan);
    }

    pub(crate) fn raise(&self, sig: crate::signal::GridSignals) {
        self.base.raise(sig);
    }

    pub(crate) fn drain_signals(&self) -> crate::signal::GridSignals {
        self.base.drain_signals()
    }

    /// Resize the backing store. Ports the guard from `CanvasRenderer::new`:
    /// only reallocates the bitmap when dimensions actually change.
    pub(crate) fn resize(&mut self, css_w: i32, css_h: i32, dpr: i32) {
        self.base.resize(css_w, css_h, dpr);
    }
}
