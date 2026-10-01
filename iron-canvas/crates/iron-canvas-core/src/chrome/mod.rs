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

use crate::CanvasSize;
use crate::geometry::CanvasMetrics;
use crate::geometry::prim::Point;
use crate::model::sheet::links::LinkIndex;
use crate::model::sheet::merges::MergeTable;
use crate::theme::CanvasTheme;

mod blit;
mod build;
mod classify;
pub mod hit;
mod kind;
pub(crate) mod merge;
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
    /// Committed link state for this frame's sheet. Empty for a sheet with no
    /// links. A candidate frame is seeded from the index captured for this
    /// attempt (`Chrome::build` and `next_blit` read it from `FrameInputs`); a
    /// held attempt hands the previously committed index back, and
    /// [`Chrome::attach_links`] installs the committed value at the completion
    /// boundary. `Rc` keeps every `Chrome` clone a refcount bump and every
    /// query reads the same committed index the pixels were painted from.
    links: Rc<LinkIndex>,
    /// Committed merge state for this frame's sheet. Empty for a sheet with no
    /// merges. Built and attached exactly like `links`: a candidate frame is
    /// seeded from the table captured for this attempt (`Chrome::build` and
    /// `next_blit` read it from `FrameInputs`); a held attempt hands the
    /// previously committed table back, and [`Chrome::attach_merges`] installs
    /// the committed value at the completion boundary. Merge geometry must
    /// commit together with the pixels that used it, so hit queries and painted
    /// pixels cannot disagree.
    merges: Rc<MergeTable>,
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

    /// The committed link index for this frame's sheet, empty when the sheet
    /// has none.
    pub fn links(&self) -> &LinkIndex {
        &self.links
    }

    /// The committed link index's shared handle. `pub(crate)` for the
    /// strategies, which must hand the committed index back to a held
    /// candidate's frame without cloning the index itself.
    pub(crate) fn links_rc(&self) -> &Rc<LinkIndex> {
        &self.links
    }

    /// Install an attempt's link index. Two callers: the strategies call it
    /// when a held candidate must hand back the previously committed index,
    /// and `Orchestrator::finish_attempt` calls it on the committed branch so
    /// the committed frame's index is exactly the captured one. Never called
    /// from `Chrome::build`/`next`/`next_blit` — those read `FrameInputs`
    /// directly, and none of them may read model metadata.
    pub(crate) fn attach_links(&mut self, links: Rc<LinkIndex>) {
        self.links = links;
    }

    /// The committed merge table for this frame's sheet, empty when the sheet
    /// has none.
    pub fn merges(&self) -> &MergeTable {
        &self.merges
    }

    /// The committed merge table's shared handle. `pub(crate)` for the
    /// strategies, which must hand the committed table back to a held
    /// candidate's frame without cloning it.
    pub(crate) fn merges_rc(&self) -> &Rc<MergeTable> {
        &self.merges
    }

    /// Install an attempt's merge table. Two callers, mirroring
    /// [`Self::attach_links`]: the strategies when a held candidate must hand
    /// back the previously committed table, and `Orchestrator::finish_attempt`
    /// on the committed branch so the committed frame's merge geometry is
    /// exactly the captured one. Never called from
    /// `Chrome::build`/`next`/`next_blit` — those read `FrameInputs` directly,
    /// and none of them may read model metadata.
    pub(crate) fn attach_merges(&mut self, merges: Rc<MergeTable>) {
        self.merges = merges;
    }
}
