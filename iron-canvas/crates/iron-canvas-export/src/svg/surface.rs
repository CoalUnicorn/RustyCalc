use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::{CanvasModel, CanvasTheme};

use super::SvgPainter;

/// `Surface` adapter wrapping `SvgPainter`. Drives `Orchestrator` for
/// one-shot SVG export. `resize` and `present` are no-ops — the SVG
/// document size is fixed at construction time and there's no backing
/// pixel buffer to flush.
pub struct SvgSurface {
    painter: Rc<SvgPainter>,
}

impl SvgSurface {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            painter: Rc::new(SvgPainter::new(width, height)),
        }
    }

    /// Drain the painter and return the finished `<svg>` document.
    /// Safe to call after the orchestrator (and its `Rc<SvgPainter>`
    /// clones) have been dropped — but also safe before, because
    /// `SvgPainter::finish` takes `&self`.
    pub fn finish(&self) -> String {
        self.painter.finish()
    }

    /// One-shot render of `model` into a self-contained `<svg>` document.
    ///
    /// Spins a throwaway `Orchestrator` over a grid + overlay
    /// `SvgSurface`, pushes `theme` and `model`, and paints a single
    /// Fresh-regime frame. Overlays (selection, marching ants,
    /// autofill handle, formula refs) are intentionally discarded: the
    /// active-cell repaint hook draws through the *overlay* surface,
    /// which is dropped here — only the grid surface's cell / border /
    /// chrome draws survive into the returned string.
    pub fn render(model: Rc<dyn CanvasModel>, theme: &CanvasTheme, size: CanvasSize) -> String {
        let width = size.w.round() as i32;
        let height = size.h.round() as i32;

        let grid = SvgSurface::new(width, height);
        let overlay = SvgSurface::new(width, height);
        let grid_painter = grid.clone_painter();

        crate::drive_once(grid, overlay, model, theme, size);

        grid_painter.finish()
    }
}

impl Surface for SvgSurface {
    type P = SvgPainter;

    fn painter(&self) -> &SvgPainter {
        self.painter.as_ref()
    }

    fn clone_painter(&self) -> Rc<SvgPainter> {
        Rc::clone(&self.painter)
    }

    /// SVG document dimensions are baked at `SvgSurface::new`; a later
    /// `resize` that disagrees would silently produce a mismatched
    /// `viewBox`. Callers must pair construction and `Orchestrator::resize`
    /// with the same `(w, h)` — the assert hardens that contract.
    fn resize(&mut self, css: CanvasSize, _dpr: f64) {
        debug_assert_eq!(
            (css.w.round() as i32, css.h.round() as i32),
            (self.painter.width, self.painter.height),
            "SvgSurface::resize disagrees with SvgPainter dimensions baked at construction",
        );
    }
    fn present(&self) {}
}
