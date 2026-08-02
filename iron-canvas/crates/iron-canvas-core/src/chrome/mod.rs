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

use crate::frame_plan::{FrameDelta, FrameInputs, RebuildReason};
use crate::geometry::{
    constants::{AUTOFILL_HANDLE_PX, CELL_AREA_INSET, HEADER_ROW_HEIGHT},
    pixel_rect::PixelRect,
    prim::Point,
    slot::scroll_first,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, RCRange};

mod blit;
mod blit_rebuild;
mod kind;
mod pane_region;
mod pane_set;
mod recycled_slots;

pub(crate) use blit::PreparedBlitFrame;
pub use blit::{BlitPlan, FramePath};
pub use kind::FrameKindTag;
pub use pane_region::{PaneRegion, PaneRegionMask};
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
    /// Canvas size at build time. `Chrome::classify` reads this to detect
    /// a resize.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. The renderer reads `frame.theme`
    /// directly; `IronCanvas::set_theme` marks both layers dirty on change,
    /// so the overlay-only fast path never paints against a stale theme.
    ///
    /// `Rc` so the per-frame snapshot is a refcount bump, not a deep clone of
    /// every color `String` — `Chrome` is rebuilt on every Fresh/SlotsReuse/
    /// Blit frame (B-1).
    pub theme: Rc<CanvasTheme>,
    /// Device pixel ratio this frame was captured with. Committed geometry
    /// metadata, not a live orchestrator read — lets `Chrome::classify`
    /// detect a DPR change by comparing committed frames only.
    pub dpr: f64,
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
    /// Build the next-frame `Chrome` for the reuse-or-rebuild regimes. The
    /// `path` argument selects which one; the body branches once and inlines
    /// the two constructors. The blit fast-path is separate
    /// ([`Self::next_blit`]) — it has a two-outcome result, not a regime tag.
    ///
    ///   * `Fresh` — full rebuild. `prev = Some` recycles slot Vec
    ///     allocations; `None` is the first-frame path. See the
    ///     [module docs](crate::chrome) for build phases A-E.
    ///   * `SlotsReuse` — prev's slot vecs and header labels survive
    ///     verbatim; only the captured per-attempt scalars (theme, dpr,
    ///     model generation, header visibility) are refreshed. Caller
    ///     refreshes overlay state separately (`SelectionLayer::refresh` in
    ///     the orchestrator).
    ///
    /// `SlotsReuse` requires `prev = Some`; `None` falls through to `Fresh`
    /// defensively. The orchestrator proves `prev.is_some()` before selecting
    /// that path, but the fallback keeps `Chrome::next` total.
    pub fn next(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        path: FramePath,
    ) -> Self {
        match path {
            FramePath::Fresh => {
                let recycled = match prev {
                    Some(c) => RecycledSlots::from_pane_set(c.pane_set),
                    None => RecycledSlots::default(),
                };
                Self::build(model, inputs, recycled)
            }
            FramePath::SlotsReuse => {
                let Some(mut prev) = prev else {
                    return Self::next(None, model, inputs, FramePath::Fresh);
                };
                // Slot vecs and header labels survive verbatim; every other
                // per-attempt scalar still refreshes from the newly captured
                // inputs so committed Chrome never lags behind the frame it
                // was actually built for. Which panes actually need
                // repainting is the caller's `GridWork` verdict — threaded
                // straight into `render_grid` as an explicit parameter, not
                // stored here.
                prev.theme = Rc::clone(inputs.theme());
                prev.dpr = inputs.dpr();
                prev.model_generation = inputs.model_generation();
                prev.show_row_headers = inputs.show_row_headers();
                prev.show_col_headers = inputs.show_col_headers();
                prev.kind = FrameKindTag::SlotsReused;
                prev
            }
        }
    }

    /// Prepare the blit fast-path's next-frame candidate without committing:
    /// `Ok(prepared)` on successful in-place reuse, `Err(prev)` — `prev`
    /// handed back whole — on reject (see `try_blit_reuse`'s doc for both
    /// cases). `Chrome::next_blit` is the immediate-commit wrapper built on
    /// top of this for callers that don't need to hold the decision open;
    /// `Orchestrator::paint_viewport_regime` calls this directly instead, so
    /// it can call `PreparedBlitFrame::rollback` if the paint that follows a
    /// successful `Ok` still fails a bulk bridge read. `pub(crate)`: an
    /// execution detail of the render pipeline, not consumer-facing API.
    // Same large-`Err` shape as `try_blit_reuse` (this just forwards to it) —
    // see that function's own comment for why `Chrome` stays by-value here
    // instead of boxed.
    #[allow(clippy::result_large_err)]
    pub(crate) fn prepare_blit(
        prev: Chrome,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: &BlitPlan,
    ) -> Result<PreparedBlitFrame, Chrome> {
        blit::try_blit_reuse(prev, model, inputs, plan)
    }

    /// Build the next-frame `Chrome` for the blit fast-path, returning a typed
    /// [`BlitOutcome`] rather than a `Chrome` with an open `FrameKindTag`.
    ///
    /// Qualification passed (`Chrome::classify` returned `FrameDelta::Scroll`),
    /// but in-place reuse may still reject — e.g. the row-header digit boundary at 99 -> 100,
    /// where `row_header_thickness` widens and the cross-axis cell-area origin
    /// shifts. `try_blit_reuse` hands `prev` back (`Err`) on reject, and we
    /// rebuild `Fresh`. The two results map straight to the two `BlitOutcome`
    /// arms at the decision point, so no caller has to assert an impossible
    /// `SlotsReused` away.
    ///
    /// Implemented through [`Self::prepare_blit`] — the same internal
    /// candidate builder `Orchestrator::paint_viewport_regime` uses — with an
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
            Ok(prepared) => BlitOutcome::Blitted(prepared.commit()),
            Err(prev) => {
                BlitOutcome::FreshFallback(Self::next(Some(prev), model, inputs, FramePath::Fresh))
            }
        }
    }

    /// Build a `FramePath::Fresh` candidate directly from a caller-supplied
    /// `recycled` pool, bypassing `Chrome::next`'s `prev`-derived recycling.
    /// `pub(crate)` so `Orchestrator::paint_fresh_regime` can build the
    /// candidate from its own standing `spare_slots` pool without handing
    /// `prev`'s ownership to this call at all — `prev` stays fully intact
    /// (and, today, still committed in `self.last_frame`) for the whole
    /// duration of the build, rather than being drained into a `RecycledSlots`
    /// derived from it as the very first step. See `chrome::recycled_slots`'s
    /// module doc for the pool's cross-attempt lifecycle.
    pub(crate) fn build(
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        recycled: RecycledSlots,
    ) -> Self {
        // `inputs` is a `FrameInputs::capture` snapshot: sheet, view, freeze
        // counts, and header visibility already read exactly once and
        // validated (a bridge failure on any of them holds the whole paint
        // attempt before `Chrome::build` is ever called — see
        // `Orchestrator::paint_if_dirty`). No fallback default is needed or
        // read here.
        let view = inputs.view();
        let sheet = inputs.sheet();

        // Visibility is modelled as thickness 0. CELL_AREA_INSET only reserves
        // the 1-px chrome border that draw_corner_box strokes; a hidden strip
        // paints no such border, so its thickness AND inset collapse to 0 and
        // cells reclaim the full edge (cell_origin follows from origin_x/_y).
        let show_row = inputs.show_row_headers();
        let show_col = inputs.show_col_headers();

        // Phase A — frozen counts, already captured.
        let frozen_row_count = inputs.frozen_rows();
        let frozen_col_count = inputs.frozen_cols();

        let mut pane_set = PaneSet::with_recycled(recycled);

        // Phase B — row walk, bounded by the model's grid (Excel's
        // LAST_ROW/LAST_COLUMN by default; finite models override).
        let canvas = inputs.size();
        let last_row = model.last_row(sheet);
        let last_column = model.last_column(sheet);
        let origin_y = if show_col {
            HEADER_ROW_HEIGHT + CELL_AREA_INSET
        } else {
            0
        };
        pane_set.fill_rows(
            model,
            sheet,
            frozen_row_count,
            origin_y,
            view.top_row,
            last_row,
            canvas.h,
        );

        // Phase C — measure row_header_thickness from the last visible row label.
        let last_visible_row = pane_set
            .rows
            .scroll
            .last()
            .map(|s| s.row)
            .unwrap_or((frozen_row_count + 1).max(view.top_row));
        let row_header_thickness = if show_row {
            measure_row_header_width(last_visible_row)
        } else {
            0
        };

        // Phase D — col walk uses the measured width to anchor `origin_x`.
        let origin_x = if show_row {
            row_header_thickness + CELL_AREA_INSET
        } else {
            0
        };
        pane_set.fill_cols(
            model,
            sheet,
            frozen_col_count,
            origin_x,
            view.left_column,
            last_column,
            canvas.w,
        );

        // Data-driven header labels in walk_header_strip (frozen ++ scroll)
        // order so header_strip can zip slots <-> labels positionally.
        pane_set.row_header_labels =
            PaneSet::resolve_row_labels(model, sheet, &pane_set.rows.frozen, &pane_set.rows.scroll);
        pane_set.col_header_labels =
            PaneSet::resolve_col_labels(model, sheet, &pane_set.cols.frozen, &pane_set.cols.scroll);

        // Phase E — assemble. `cell_origin` reuses the locals from B/D so
        // there's a single source of truth for the cell-area top-left.
        let col_header_thickness = if show_col { HEADER_ROW_HEIGHT } else { 0 };
        let cell_origin = Point {
            x: origin_x,
            y: origin_y,
        };

        Chrome {
            sheet,
            pane_set,
            row_header_thickness,
            col_header_thickness,
            cell_origin,
            canvas_size: canvas,
            theme: Rc::clone(inputs.theme()),
            dpr: inputs.dpr(),
            model_generation: inputs.model_generation(),
            show_row_headers: show_row,
            show_col_headers: show_col,
            kind: FrameKindTag::Fresh,
        }
    }

    /// The one classifier that replaces the former split verdict
    /// (`FrameValidity` from `is_still_valid`, `Option<BlitPlan>` from
    /// `screen_for_blit`) with a single three-way [`FrameDelta`].
    /// `Chrome::next`/`Chrome::next_blit`'s own dispatch is unchanged; the
    /// orchestrator passes this verdict through `plan_frame` to the regime
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
    /// `Chrome`, mutates cache state, calls a painter, or fetches pane
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
        if inputs.size() != prev.canvas_size {
            return FrameDelta::Rebuild(RebuildReason::Size);
        }
        if inputs.dpr() != prev.dpr {
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
    /// oversized selections to the canvas edge. `None` when the range
    /// lies entirely outside the drawable fold. Pure `Chrome` math, no
    /// model access.
    pub fn range_rect(&self, range: RCRange) -> Option<PixelRect> {
        let p = &self.pane_set;
        let frozen_rows = p.rows.frozen_count();
        let frozen_cols = p.cols.frozen_count();

        if !self.range_intersects_fold(range, frozen_rows, frozen_cols) {
            return None;
        }

        let x = p.col_to_x(range.c1);
        let y = p.row_to_y(range.r1);
        let right = if range.c2 > p.last_visible_col() && range.c2 > frozen_cols {
            self.canvas_size.w as i32
        } else {
            p.col_to_x(range.c2) + p.col_extent_at(range.c2)
        };
        let bottom = if range.r2 > p.last_visible_row() && range.r2 > frozen_rows {
            self.canvas_size.h as i32
        } else {
            p.row_to_y(range.r2) + p.row_extent_at(range.r2)
        };
        Some(PixelRect {
            top_left: Point { x, y },
            width: right - x,
            height: bottom - y,
        })
    }

    /// True if `range` overlaps the drawable fold (scrollable viewport
    /// plus the frozen bands). Guards `range_rect`'s slot lookups against
    /// off-screen refs like `=BB3` when column BB is not visible.
    fn range_intersects_fold(&self, range: RCRange, frozen_rows: i32, frozen_cols: i32) -> bool {
        let p = &self.pane_set;
        if range.c1 > p.last_visible_col() && range.c1 > frozen_cols {
            return false;
        }
        if range.r1 > p.last_visible_row() && range.r1 > frozen_rows {
            return false;
        }
        if range.c2 < p.left_column() && range.c2 > frozen_cols {
            return false;
        }
        if range.r2 < p.top_row() && range.r2 > frozen_rows {
            return false;
        }
        true
    }

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
