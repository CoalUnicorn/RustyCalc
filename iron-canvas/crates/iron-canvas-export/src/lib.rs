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
    iron_canvas_core::geometry::{CanvasMetricError, CanvasMetrics},
    iron_canvas_core::layer::Surface,
    iron_canvas_core::{CanvasModel, CanvasTheme, Orchestrator, PaintResult},
    std::rc::Rc,
};

/// Why a one-shot export produced no document.
///
/// The export path is one paint attempt: a held attempt leaves the grid with
/// partial or no pixels, so the honest result is an error rather than a
/// document that silently misses content.
#[cfg(any(feature = "svg", feature = "pdf"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportError {
    /// The requested canvas size cannot be a canvas metric pair (see
    /// [`CanvasMetricError`]): non-finite, negative, or too large to back.
    Metrics(CanvasMetricError),
    /// The single paint attempt did not commit a frame — a scalar or bulk
    /// model read failed, so the pixels are not trustworthy. A host that
    /// wants the document can retry once the model is readable again.
    RetryRequired,
}

#[cfg(any(feature = "svg", feature = "pdf"))]
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Metrics(e) => write!(f, "invalid export canvas metrics: {e}"),
            Self::RetryRequired => {
                f.write_str("export paint attempt did not commit a frame; retry required")
            }
        }
    }
}

#[cfg(any(feature = "svg", feature = "pdf"))]
impl std::error::Error for ExportError {}

#[cfg(any(feature = "svg", feature = "pdf"))]
impl From<CanvasMetricError> for ExportError {
    fn from(error: CanvasMetricError) -> Self {
        Self::Metrics(error)
    }
}

/// Whole-pixel document dimensions for a one-shot export.
///
/// Both backend surfaces bake these into their document (`/MediaBox`,
/// `viewBox`) at construction and assert against them in `resize`, so the
/// conversion lives here rather than once per backend. The size is already
/// validated by [`CanvasMetrics::new`], so neither cast can overflow.
/// Rounds logical CSS dimensions to the nearest whole pixel. Fractions below
/// 0.5 round down; other fractions round up. This preserves the export size
/// policy. [`CanvasMetrics::backing_size`] instead scales by DPR and truncates.
#[cfg(any(feature = "svg", feature = "pdf"))]
pub(crate) fn document_size(metrics: CanvasMetrics) -> (u32, u32) {
    let size = metrics.size();
    (size.w.round() as u32, size.h.round() as u32)
}

/// Drive a throwaway `Orchestrator` for a single one-shot export frame.
///
/// Captures the one ordered sequence both `SvgSurface::render` and
/// `PdfSurface::render` repeat (`new → set_theme → set_model → resize →
/// render_pending → drop`) — the only drift surface between the two
/// backends. No explicit `request_repaint` is needed: `set_model` already
/// discards any queued work and installs a fresh `geometry + content(ALL) +
/// overlay` value, and `resize` drops `last_frame` and marks geometry too —
/// between them the one Fresh frame this export needs is already queued.
/// Policy-neutral: the helper never finishes a surface, so the
/// overlay-discard decision stays with the caller, which pre-clones the
/// *grid* handle and never reads the overlay.
///
/// `metrics` is already parsed by the caller (which also sized its painters
/// from it). Returns [`ExportError::RetryRequired`] when the attempt did not
/// paint — an `Idle` result here would mean the helper's own queued Fresh
/// work vanished, which is as unusable to a caller as a held attempt.
#[cfg(any(feature = "svg", feature = "pdf"))]
pub(crate) fn drive_once<S: Surface>(
    grid: S,
    overlay: S,
    model: Rc<dyn CanvasModel>,
    theme: &CanvasTheme,
    metrics: CanvasMetrics,
) -> Result<(), ExportError> {
    let mut orchestrator = Orchestrator::new(grid, overlay);
    orchestrator.set_theme(theme.clone());
    orchestrator.set_model(model);
    orchestrator.resize(metrics);
    match orchestrator.render_pending() {
        PaintResult::Rendered => Ok(()),
        PaintResult::Idle | PaintResult::RetryRequired => Err(ExportError::RetryRequired),
    }
    // `orchestrator` (and its `Rc<P>` surface clones) drop here; the caller's
    // pre-cloned grid painter/stream survives to `finish()` / `build_document`.
}
