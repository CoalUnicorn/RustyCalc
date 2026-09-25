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

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

pub use crate::frame::{BlitPlan, Shift};
use crate::frame::{FrameDelta, FrameInputs, RebuildReason};
use crate::geometry::CanvasMetrics;
use crate::geometry::{
    constants::AUTOFILL_HANDLE_PX, pixel_rect::PixelRect, prim::Point, slot::scroll_first,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, RCRange};

mod blit;
mod build;
mod kind;
mod pane_region;
mod pane_set;
mod recycled_slots;

pub(crate) use blit::PreparedBlitOutcome;
pub use build::FramePath;
pub(crate) use build::FreshBuild;
pub use kind::FrameKindTag;
pub use pane_region::{GridLayout, GridSegment, GridShape, PaneRegion};
pub use pane_set::{PaneSet, measure_row_header_width};
pub use recycled_slots::RecycledSlots;

/// In-process digest of a formatted cell value. `DefaultHasher` output is
/// only stable within one std version, so the digest must never be
/// persisted or compared across builds. The newtype shape blocks
/// accidental serialization / cross-process comparison at the type
/// system level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellValueHash(u64);

/// Snapshot of the active cell at paint time. `Chrome::classify` re-hashes
/// the live model's value at the stored coords; a mismatch means the cell
/// was edited since this `Chrome` was painted, and the blit's kept band
/// would carry stale pixels. Catches the canonical edit-then-scroll case
/// without requiring the consumer to call `markContentDirty`.
#[derive(Clone, Debug, Default)]
pub struct ActiveCellSnapshot {
    pub row: i32,
    pub col: i32,
    /// `None` when the fetch was `BridgeFailed` — an *unknown* value, distinct
    /// from a known-empty (`Absent`) cell which hashes as `""`. An unknown on
    /// either side of `matches` can't prove the cell is unchanged, so it must
    /// reject the blit rather than blit stale pixels.
    pub value_hash: Option<CellValueHash>,
}

// `None` for `BridgeFailed` (value unknown); `Absent` is a known-empty cell and
// hashes as `""` so it stays comparable across frames.
fn hash_cell_value(
    model: &dyn CanvasModel,
    sheet: u32,
    row: i32,
    col: i32,
) -> Option<CellValueHash> {
    let fetched = model.get_formatted_cell_value(sheet, row, col);
    if fetched.is_bridge_failed() {
        return None;
    }
    let value = fetched.value().unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    Some(CellValueHash(hasher.finish()))
}

impl ActiveCellSnapshot {
    pub fn capture(model: &dyn CanvasModel, sheet: u32, row: i32, col: i32) -> Self {
        Self {
            row,
            col,
            value_hash: hash_cell_value(model, sheet, row, col),
        }
    }

    pub fn matches(&self, model: &dyn CanvasModel, sheet: u32) -> bool {
        // Blit only when both fetches are known AND equal. A `BridgeFailed`
        // (`None`) at capture or compare time means "can't prove unchanged" ->
        // reject, forcing a fresh repaint instead of reusing stale pixels.
        match (
            self.value_hash,
            hash_cell_value(model, sheet, self.row, self.col),
        ) {
            (Some(captured), Some(live)) => captured == live,
            _ => false,
        }
    }
}

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

    /// The one classifier that replaces the former split verdict
    /// (`FrameValidity` from `is_still_valid`, `Option<BlitPlan>` from
    /// `screen_for_blit`) with a single three-way [`FrameDelta`].
    /// `Chrome::next`/`Chrome::next_blit`'s own dispatch is unchanged; the
    /// orchestrator passes this verdict through `plan_frame` to the strategy
    /// executor.
    ///
    /// `prev = None` (no committed frame — right after a `resize`, or
    /// before the first paint) is itself the first ordered comparison, not
    /// a caller precondition. Every later comparison assumes a committed
    /// frame to compare against, in this fixed order (stable so a given
    /// divergence always reports under the same `RebuildReason`, even when
    /// several fields changed in the same tick):
    ///
    /// 1. no committed frame -> `NoCommittedFrame`
    /// 2. canvas size -> `Size`
    /// 3. DPR -> `Dpr`
    /// 4. theme -> `Theme`
    /// 5. model generation -> `Model`
    /// 6. sheet -> `Sheet`
    /// 7. frozen row/column counts -> `Freeze`
    /// 8. header visibility -> `Headers`
    /// 9. effective top/left scroll origin (see below)
    ///
    /// Steps 1-8 are hard breaks — any divergence rebuilds outright, the
    /// same theme/canvas-size/sheet/freeze rejections `is_still_valid` and
    /// `screen_for_blit` used to duplicate across two functions, plus the
    /// DPR/model-generation/header comparisons neither of them made.
    ///
    /// Step 9 is not itself a hard break; it forks the geometric scroll
    /// question:
    ///
    /// - neither axis moved -> `Stable` (slot vecs, and everything painted
    ///   from them, are reusable as-is);
    /// - both axes moved -> `Rebuild(TwoAxisScroll)` (no single blit shift
    ///   expresses a diagonal scroll);
    /// - exactly one axis moved:
    ///   - no committed active-cell snapshot to re-hash against ->
    ///     `Rebuild(MissingActiveSnapshot)`;
    ///   - the snapshot's value no longer matches the live model
    ///     (edit-then-scroll), or either read is unknown (`BridgeFailed`) ->
    ///     `Rebuild(ActiveCellChangedOrUnknown)`;
    ///   - the axis-specific overlap probe finds a safe kept band ->
    ///     `Scroll(plan)`;
    ///   - otherwise -> `Rebuild(IncompatibleScrollOverlap)`.
    ///
    /// Reads `model` only for the two checks that were always live-model
    /// reads, never part of `FrameInputs`: the active-cell re-hash and the
    /// overlap probe's row-height/col-width lookups. Never builds a
    /// `Chrome`, mutates cache state, calls a painter, or fetches cell
    /// content — qualification only. Construction (and its own independent
    /// reject, e.g. the row-header digit boundary) stays in
    /// `Chrome::next_blit`.
    pub fn classify(
        prev: Option<&Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        active_cell: Option<&ActiveCellSnapshot>,
    ) -> FrameDelta {
        let Some(prev) = prev else {
            return FrameDelta::Rebuild(RebuildReason::NoCommittedFrame);
        };
        if inputs.size() != prev.canvas_size() {
            return FrameDelta::Rebuild(RebuildReason::Size);
        }
        if inputs.dpr() != prev.dpr() {
            return FrameDelta::Rebuild(RebuildReason::Dpr);
        }
        if inputs.theme() != &prev.theme {
            return FrameDelta::Rebuild(RebuildReason::Theme);
        }
        if inputs.model_generation() != prev.model_generation {
            return FrameDelta::Rebuild(RebuildReason::Model);
        }
        let sheet = inputs.sheet();
        if sheet != prev.sheet {
            return FrameDelta::Rebuild(RebuildReason::Sheet);
        }
        let frozen_rows = inputs.frozen_rows();
        let frozen_cols = inputs.frozen_cols();
        if frozen_rows != prev.pane_set.rows.frozen_count()
            || frozen_cols != prev.pane_set.cols.frozen_count()
        {
            return FrameDelta::Rebuild(RebuildReason::Freeze);
        }
        if inputs.show_row_headers() != prev.show_row_headers
            || inputs.show_col_headers() != prev.show_col_headers
        {
            return FrameDelta::Rebuild(RebuildReason::Headers);
        }

        let view = inputs.view();
        let new_top = scroll_first(frozen_rows, view.top_row);
        let new_left = scroll_first(frozen_cols, view.left_column);
        let top_changed = new_top != prev.pane_set.top_row();
        let left_changed = new_left != prev.pane_set.left_column();

        match (top_changed, left_changed) {
            (false, false) => FrameDelta::Stable,
            (true, true) => FrameDelta::Rebuild(RebuildReason::TwoAxisScroll),
            _ => {
                // Defensive content check: if the cell painted as active no
                // longer matches the live model, the blit's kept band would
                // shift pre-edit pixels (canonical edit-then-scroll bug when
                // the consumer missed `markContentDirty`). An absent snapshot
                // (nothing captured yet this attempt) can't be re-hashed at
                // all, so it gets its own, more specific reason.
                let Some(active) = active_cell else {
                    return FrameDelta::Rebuild(RebuildReason::MissingActiveSnapshot);
                };
                if !active.matches(model, sheet) {
                    return FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown);
                }
                let plan = if top_changed {
                    blit::try_blit_rows(prev, model, sheet, new_top)
                } else {
                    blit::try_blit_cols(prev, model, sheet, new_left)
                };
                match plan {
                    Some(plan) => FrameDelta::Scroll(plan),
                    None => FrameDelta::Rebuild(RebuildReason::IncompatibleScrollOverlap),
                }
            }
        }
    }

    pub fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        let p = &self.pane_set;
        if !p.row_in_frame(row) || !p.col_in_frame(col) {
            return None;
        }
        Some(PixelRect {
            top_left: Point {
                x: p.col_to_x(col),
                y: p.row_to_y(row),
            },
            width: p.col_extent_at(col),
            height: p.row_extent_at(row),
        })
    }

    /// Map a sheet-coordinate range to canvas pixel bounds, clamping
    /// oversized selections to the canvas edge. `None` when no row or no
    /// column of the range is painted in this frame — the range lies entirely
    /// in the address gap between the frozen band and the scrolled-to band, or
    /// beyond the walked extent. Pure `Chrome` math, no model access.
    ///
    /// Both axes are normalized once, then projected onto their
    /// frozen-plus-scroll union
    /// ([`AxisSlots::project_interval`](crate::geometry::slot::AxisSlots::project_interval)). A range that
    /// starts in the address gap and ends in the scroll band therefore covers
    /// only the ids it names — never the header or frozen pixels between them —
    /// and a range that overlaps only the frozen band still returns its true
    /// rectangle instead of a zero-extent one built from off-frame zeros.
    pub fn range_rect(&self, range: RCRange) -> Option<PixelRect> {
        let norm = range.normalized();
        let p = &self.pane_set;
        let (canvas_w, canvas_h) = self.metrics.logical_extent();

        let (x, mut right) = p.cols.project_interval(norm.c1, norm.c2)?;
        let (y, mut bottom) = p.rows.project_interval(norm.r1, norm.r2)?;

        // The selection continues past the last painted id: there is no slot to
        // take a trailing edge from, so the outline runs to the canvas edge.
        if norm.c2 > p.cols.frozen_count().max(p.cols.last_visible()) {
            right = canvas_w;
        }
        if norm.r2 > p.rows.frozen_count().max(p.rows.last_visible()) {
            bottom = canvas_h;
        }
        Some(PixelRect {
            top_left: Point { x, y },
            // A frozen band taller/wider than the canvas can start past the
            // clamped edge; never hand a painter a negative extent.
            width: (right - x).max(0),
            height: (bottom - y).max(0),
        })
    }

    /// Return the bottom-right corner of the selection's last cell in canvas
    /// pixels. Return `None` if either cell coordinate has no frame slot or
    /// reaches the model's last row or column. A retained edge slot can extend
    /// past the canvas, so the returned point is not clipped to the canvas.
    pub fn autofill_handle(&self, selection_range: RCRange) -> Option<Point> {
        let norm = selection_range.normalized();
        let r2 = norm.r2;
        let c2 = norm.c2;
        let p = &self.pane_set;
        // Selections reaching the grid's last row/column (full-row,
        // full-column, or up against a finite model's data boundary) get
        // no handle — there is nothing beyond to fill into.
        if r2 >= p.rows.last_id || c2 >= p.cols.last_id {
            return None;
        }
        if !p.row_in_frame(r2) || !p.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: p.col_to_x(c2) + p.col_extent_at(c2),
            y: p.row_to_y(r2) + p.row_extent_at(r2),
        })
    }

    /// Return the handle's fill rectangle, with [`AUTOFILL_HANDLE_PX`] per side.
    /// Its bottom-right corner is [`autofill_handle`](Chrome::autofill_handle).
    /// The rectangle can extend beyond a cell smaller than the handle.
    /// The selection painter strokes a separate outline around this rectangle.
    /// Return `None` under the same conditions as the anchor.
    pub fn autofill_handle_rect(&self, selection_range: RCRange) -> Option<PixelRect> {
        let p = self.autofill_handle(selection_range)?;
        Some(PixelRect {
            top_left: Point {
                x: p.x - AUTOFILL_HANDLE_PX,
                y: p.y - AUTOFILL_HANDLE_PX,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        })
    }

    pub fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < self.cell_origin.x && y < self.cell_origin.y {
            return HitTest::Corner;
        }
        let p = &self.pane_set;
        if y < self.cell_origin.y {
            return match p.cols.pixel_to_id(x) {
                Some(c) => HitTest::ColumnHeader(c),
                None => HitTest::Outside,
            };
        }
        if x < self.cell_origin.x {
            return match p.rows.pixel_to_id(y) {
                Some(r) => HitTest::RowHeader(r),
                None => HitTest::Outside,
            };
        }
        let (Some(row), Some(column)) = (p.rows.pixel_to_id(y), p.cols.pixel_to_id(x)) else {
            return HitTest::Outside;
        };
        // `AutofillHandle` is resolved by `AutofillLayer::hit_test` in
        // the orchestrator's reverse-z walk; the grid path returns plain
        // cell / header / corner / outside.
        HitTest::Cell { row, column }
    }

    pub fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < self.col_header_thickness && x > self.row_header_thickness {
            return self
                .pane_set
                .cols
                .boundary_at(x, tolerance)
                .map(ResizeTarget::ColumnEdge);
        }
        if x < self.row_header_thickness && y > self.col_header_thickness {
            return self
                .pane_set
                .rows
                .boundary_at(y, tolerance)
                .map(ResizeTarget::RowEdge);
        }
        None
    }
}
