use wasm_bindgen::JsValue;
use web_sys::HtmlCanvasElement;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::{LayerBase, Surface};

use crate::canvas_painter::CanvasPainter;
use crate::chrome::{BlitPlan, Chrome};
use crate::renderer::GridRenderer;
use crate::web_surface::WebSurface;
use crate::CanvasModel;

pub(crate) struct GridLayer {
    base: LayerBase<WebSurface, GridRenderer<CanvasPainter>>,
}

impl GridLayer {
    pub(crate) fn create(canvas: HtmlCanvasElement) -> Result<Self, JsValue> {
        let surface = WebSurface::grid(canvas)?;
        let renderer = GridRenderer::for_layer(surface.clone_painter());
        Ok(Self {
            base: LayerBase::new(surface, renderer),
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
        // The raw `ctx.set_fill_style_str` below bypasses the painter
        // cache but doesn't read it, so an Empty cache here is safe —
        // the first cached-set inside `render_grid` re-binds from Empty.
        if !frame.kind.reuses_slots() {
            let size = frame.canvas_size;
            let ctx = self.base.surface.painter().ctx();
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
        self.base.resize(
            CanvasSize {
                w: f64::from(css_w),
                h: f64::from(css_h),
            },
            dpr,
        );
    }
}
