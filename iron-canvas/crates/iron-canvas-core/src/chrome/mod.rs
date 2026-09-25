//! Per-frame snapshot of painted chrome geometry. The renderer and every
//! `Orchestrator` query read the same `Chrome`, so painted pixels and hit
//! zones cannot disagree.
//!
//! Pure-axis walks live on [`PaneSet`]; `Chrome` composes them whenever a
//! query spans both axes.
//!
//! # Build phases
//!
//! `FramePath::Fresh` runs the private `Chrome::build` in five fixed-order
//! phases. The order is load-bearing: phase C measures a value phase D needs,
//! and both axis walks must finish before E assembles the shared `cell_origin`.
//!
//! ```text
//! A  frozen counts   inputs.frozen_rows() / inputs.frozen_cols()
//! B  row walk        PaneSet::with_recycled(recycled).fill_rows(..)
//! C  measure r.h.t.  row_header_thickness = measure_row_header_width(last_visible_row)
//! D  col walk        pane_set.fill_cols(..)   // origin_x = row_header_thickness + CELL_AREA_INSET
//! E  assemble        Chrome { pane_set, row_header_thickness, cell_origin, .. }
//! ```
//!
//! `SlotsReuse` skips the walk: it keeps the previous slot vecs and refreshes
//! only per-frame state. `Chrome::classify` decides between `Stable` (skip
//! the walk entirely), `Scroll` (blit fast-path), and `Rebuild` (full
//! `Fresh` walk) by comparing the previous frame's committed geometry
//! metadata against the newly captured `FrameInputs`; any hard-break
//! divergence — or a scroll with no safe kept overlap — forces `Rebuild`.

use std::rc::Rc;

use crate::frame::FrameInputs;
pub use crate::frame::{BlitPlan, Shift};
use crate::geometry::CanvasMetrics;
use crate::geometry::prim::Point;
use crate::theme::CanvasTheme;
use crate::{CanvasModel, CanvasSize};

mod blit;
mod build;
mod classify;
mod kind;
mod pane_region;
mod pane_set;
mod query;
mod recycled_slots;

pub(crate) use blit::PreparedBlitOutcome;
pub use build::FramePath;
pub(crate) use build::FreshBuild;
pub use classify::{ActiveCellSnapshot, CellValueHash};
pub use kind::FrameKindTag;
pub use pane_region::{GridLayout, GridSegment, GridShape, PaneRegion};
pub use pane_set::{PaneSet, measure_row_header_width};
pub use recycled_slots::RecycledSlots;

#[derive(Debug, Clone)]
pub struct Chrome {
    pub sheet: u32,
    pub pane_set: PaneSet,
    /// Measured per frame from the widest visible row label.
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    /// Top-left of the cell area; single source of truth for hit-test
    /// and viewport math.
    pub cell_origin: Point,
    /// Canvas metrics at build time. `Chrome::classify` reads this to detect
    /// a resize or DPR change, and every geometry walk takes its logical extent
    /// and backing size from here. Private: the value carries
    /// [`CanvasMetrics`]' validated invariant, and `Chrome::next`/`next_blit`
    /// are the only producers.
    metrics: CanvasMetrics,
    /// Theme this frame was painted with. The renderer reads `frame.theme`
    /// directly; `IronCanvas::set_theme` marks both layers dirty on change,
    /// so the overlay-only fast path never paints against a stale theme.
    ///
    /// `Rc` so the per-frame snapshot is a refcount bump, not a deep clone of
    /// every color `String` — `Chrome` is rebuilt on every Fresh/SlotsReuse/
    /// Blit frame (B-1).
    pub theme: Rc<CanvasTheme>,
    /// `Orchestrator::model_generation` at capture time. Committed so
    /// `Chrome::classify` can detect a `set_model` replacement without
    /// comparing trait-object pointers.
    pub model_generation: u64,
    /// Row/column header visibility captured with this frame. Both already
    /// determine `row_header_thickness`/`col_header_thickness` at build
    /// time; committing the source booleans too keeps them available to
    /// `Chrome::classify` without re-deriving them from thickness alone.
    pub show_row_headers: bool,
    pub show_col_headers: bool,
    /// Which constructor produced this frame. Renderer diagnostics and
    /// paint-skip gating read it; `FrameKindTag::reuses_slots()` is the
    /// "slot vecs inherited from prev" predicate.
    pub kind: FrameKindTag,
}

/// Outcome of [`Chrome::next_blit`]. The blit construction has exactly two
/// results — in-place reuse succeeded, or it rejected and fell back to a full
/// rebuild — so they are *variants*, not a tag the caller has to assert one
/// case away from. Each carries the built `Chrome`; the caller dispatches the
/// paint (blit copy vs full repaint) on which arm it got.
#[must_use = "the built Chrome must become the next last_frame"]
pub enum BlitOutcome {
    /// In-place reuse succeeded: the kept band was blitted, only the strip
    /// touched the model. Caller paints via `paint_grid_blit`.
    Blitted(Chrome),
    /// Reuse rejected (e.g. row-header digit-boundary 99 -> 100) and the frame
    /// was rebuilt `Fresh`. Caller invalidates caches and paints `paint_grid`.
    FreshFallback(Chrome),
}

impl Chrome {
    /// Validated canvas metrics this frame was built with.
    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics
    }

    /// Logical canvas size at build time.
    pub fn canvas_size(&self) -> CanvasSize {
        self.metrics.size()
    }

    /// Device pixel ratio this frame was captured with. Committed geometry
    /// metadata, not a live orchestrator read — lets `Chrome::classify`
    /// detect a DPR change by comparing committed frames only.
    pub fn dpr(&self) -> f64 {
        self.metrics.dpr()
    }

    /// Piecewise address layout for the visible grid.
    pub fn grid_layout(&self) -> GridLayout {
        GridLayout::from_frame(self)
    }

    /// Prepare the blit fast-path's next-frame candidate without committing:
    /// [`PreparedBlitOutcome::Ready`] on successful in-place reuse,
    /// [`PreparedBlitOutcome::FreshFallback`] carrying `prev` whole on reject
    /// (see `try_blit_reuse`'s doc for both cases). `Chrome::next_blit` is the
    /// immediate-commit wrapper built on top of this for callers that don't
    /// need to hold the decision open; `Orchestrator::render_scroll_blit`
    /// calls this directly instead, so it can call
    /// `PreparedBlitFrame::rollback` if the paint that follows a successful
    /// `Ready` still fails a bulk bridge read. `pub(crate)`: an execution
    /// detail of the render pipeline, not consumer-facing API.
    pub(crate) fn prepare_blit(
        prev: Chrome,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: &BlitPlan,
    ) -> PreparedBlitOutcome {
        blit::try_blit_reuse(prev, model, inputs, plan)
    }

    /// Build the next-frame `Chrome` for the blit fast-path, returning a typed
    /// [`BlitOutcome`] rather than a `Chrome` with an open `FrameKindTag`.
    ///
    /// Qualification passed (`Chrome::classify` returned `FrameDelta::Scroll`),
    /// but in-place reuse may still reject — e.g. the row-header digit boundary at 99 -> 100,
    /// where `row_header_thickness` widens and the cross-axis cell-area origin
    /// shifts. `try_blit_reuse` hands `prev` back
    /// (`PreparedBlitOutcome::FreshFallback`) on reject, and we rebuild
    /// `Fresh`. The two outcomes map straight to the two `BlitOutcome` arms at
    /// the decision point, so no caller has to assert an impossible
    /// `SlotsReused` away.
    ///
    /// Implemented through [`Self::prepare_blit`] — the same internal
    /// candidate builder `Orchestrator::render_scroll_blit` uses — with an
    /// immediate `.commit()`: there is no second blit construction algorithm,
    /// only a second (non-atomic) way to consume the first one's result.
    pub fn next_blit(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: &BlitPlan,
    ) -> BlitOutcome {
        let Some(prev) = prev else {
            return BlitOutcome::FreshFallback(Self::next(None, model, inputs, FramePath::Fresh));
        };
        match Self::prepare_blit(prev, model, inputs, plan) {
            PreparedBlitOutcome::Ready(prepared) => BlitOutcome::Blitted(prepared.commit()),
            PreparedBlitOutcome::FreshFallback(prev) => {
                BlitOutcome::FreshFallback(Self::next(Some(prev), model, inputs, FramePath::Fresh))
            }
        }
    }
}
