//! Multi-format export backend for `iron-canvas`.
//!
//! Each format lives behind its own feature flag and contributes a
//! `Painter + BlitPainter + TextMetrics` adapter plus a `Surface` impl
//! that drives a throwaway `Orchestrator`.

pub mod common;

#[cfg(feature = "svg")]
pub mod svg;

#[cfg(feature = "svg")]
pub use svg::{SvgPainter, SvgSurface};

#[cfg(feature = "pdf")]
pub mod pdf;

#[cfg(feature = "pdf")]
pub use pdf::{PdfPainter, PdfSurface};

#[cfg(any(feature = "svg", feature = "pdf"))]
use {
    iron_canvas_core::geometry::CanvasSize,
    iron_canvas_core::layer::Surface,
    iron_canvas_core::{CanvasModel, CanvasTheme, Orchestrator},
    std::rc::Rc,
};

/// Drive a throwaway `Orchestrator` for a single one-shot export frame.
///
/// Captures the one ordered sequence both `SvgSurface::render` and
/// `PdfSurface::render` repeat (`new → set_theme → set_model → resize →
/// request_repaint → paint_if_dirty → drop`) — the only drift surface between
/// the two backends. Policy-neutral: the helper never finishes a surface, so
/// the overlay-discard decision stays with the caller, which pre-clones the
/// *grid* handle and never reads the overlay.
#[cfg(any(feature = "svg", feature = "pdf"))]
pub(crate) fn drive_once<S: Surface>(
    grid: S,
    overlay: S,
    model: Rc<dyn CanvasModel>,
    theme: &CanvasTheme,
    size: CanvasSize,
) {
    let mut orchestrator = Orchestrator::new(grid, overlay);
    orchestrator.set_theme(theme.clone());
    orchestrator.set_model(model);
    orchestrator.resize(size, 1);
    orchestrator.request_repaint();
    orchestrator.paint_if_dirty();
    // `orchestrator` (and its `Rc<P>` surface clones) drop here; the caller's
    // pre-cloned grid painter/stream survives to `finish()` / `build_document`.
}
