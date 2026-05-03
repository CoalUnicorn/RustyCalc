mod grid;
mod overlay;

pub(crate) use grid::GridLayer;
pub(crate) use overlay::OverlayLayer;
use web_sys::HtmlCanvasElement;

use crate::{CanvasRenderer, CanvasSize};

pub(crate) struct PaintGate {
    dirty: bool,
    #[cfg(test)]
    pub(crate) paint_count: u32,
}

pub(crate) struct LayerBase {
    pub(crate) canvas: HtmlCanvasElement,
    gate: PaintGate,
    pub(crate) renderer: CanvasRenderer,
}

impl LayerBase {
    pub(crate) fn new(canvas: HtmlCanvasElement, renderer: CanvasRenderer) -> Self {
        Self {
            canvas,
            gate: PaintGate::new(),
            renderer,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.gate.mark_dirty();
    }

    /// Consume the dirty flag. Returns `true` if a paint is needed.
    pub(crate) fn should_paint(&mut self) -> bool {
        self.gate.should_paint()
    }

    pub(crate) fn canvas_size(&self) -> CanvasSize {
        self.renderer.canvas_size()
    }

    pub(crate) fn resize(&mut self, css_w: f64, css_h: f64, dpr: f64) {
        let (target_w, target_h) = CanvasSize { w: css_w, h: css_h }.to_backing_size(dpr);
        if self.canvas.width() != target_w || self.canvas.height() != target_h {
            self.canvas.set_width(target_w);
            self.canvas.set_height(target_h);
        } else {
            self.renderer
                .ctx_ref()
                .set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .expect("set_transform should not fail");
        }
        self.renderer
            .ctx_ref()
            .scale(dpr, dpr)
            .expect("scale should not fail");
        self.renderer.set_size(css_w, css_h);
        self.renderer.set_dpr(dpr);

        // Note: keep calling it in both paths for now;
        // tighten in a follow-up if profiling warrants.
        self.renderer.invalidate_paint_cache();
    }
}

impl PaintGate {
    pub(crate) fn new() -> Self {
        Self {
            dirty: false,
            #[cfg(test)]
            paint_count: 0,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        let was_dirty = std::mem::replace(&mut self.dirty, false);
        #[cfg(test)]
        if was_dirty {
            self.paint_count += 1;
        }
        was_dirty
    }
}
