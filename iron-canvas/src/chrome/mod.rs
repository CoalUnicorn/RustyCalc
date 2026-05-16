//! Per-frame snapshot of painted chrome geometry. The renderer and every
//! `IronCanvas` query read the same `Chrome`, so painted pixels and hit
//! zones cannot disagree.
//!
//! Pure-axis walks live on `PaneSet`; `Chrome` composes them whenever a
//! query spans both axes. See `ARCHITECTURE.md` for the build phases
//! (A–E) and the `is_still_valid` cache rules.

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::geometry::slot::scroll_first;
use crate::geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN,
        LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::Point,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, RCRange};

mod blit;
pub(crate) mod kind;
pub(crate) mod pane_region;
mod pane_set;

pub(crate) use blit::{BlitPlan, FramePath};
pub(crate) use kind::FrameKindTag;
pub(crate) use pane_region::{PaneRegion, PaneRegionMask};
pub(crate) use pane_set::{measure_row_header_width, PaneSet, RecycledSlots};

/// Snapshot of the active cell at paint time. `try_blit` re-hashes the
/// live model's value at the stored coords; a mismatch means the cell
/// was edited since this `Chrome` was painted, and the blit's kept band
/// would carry stale pixels. Catches the canonical edit-then-scroll case
/// without requiring the consumer to call `markContentDirty`.
#[derive(Clone, Debug)]
pub(crate) struct ActiveCellSnapshot {
    pub row: i32,
    pub col: i32,
    pub value_hash: u64,
}

impl ActiveCellSnapshot {
    pub(crate) fn capture(model: &dyn CanvasModel, sheet: u32, row: i32, col: i32) -> Self {
        let value = model
            .get_formatted_cell_value(sheet, row, col)
            .unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        Self {
            row,
            col,
            value_hash: hasher.finish(),
        }
    }

    pub(crate) fn matches(&self, model: &dyn CanvasModel, sheet: u32) -> bool {
        let value = model
            .get_formatted_cell_value(sheet, self.row, self.col)
            .unwrap_or_default();
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish() == self.value_hash
    }
}

#[derive(Debug)]
pub(crate) struct Chrome {
    pub sheet: u32,
    pub pane_set: PaneSet,
    /// Measured per frame from the widest visible row label.
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    /// Top-left of the cell area; single source of truth for hit-test
    /// and viewport math.
    pub cell_origin: Point,
    /// Selection snapshot at paint time, raw `[r1, c1, r2, c2]` from
    /// `SelectedView.range`. Pins `autofill_handle` to the painted
    /// selection even if the model's selection has since moved.
    ///
    /// Invariant: no paint or hit-test code reads
    /// `model.get_selected_view().selection`; every consumer goes through
    /// this field, so painted and queried geometry stay in lockstep.
    pub selection_range: RCRange,
    /// Active-cell coords + value hash at paint time. `try_blit` rejects
    /// when the live model's value at these coords no longer matches,
    /// catching edit-then-scroll cases where the consumer missed
    /// `markContentDirty`. Refreshed by `refresh_overlay_inputs` on
    /// SlotsReuse paths so selection-only moves don't poison the check.
    pub(crate) active_cell: ActiveCellSnapshot,
    /// Canvas size at build time. `is_still_valid` reads this to detect
    /// a resize.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. The renderer reads `frame.theme`
    /// directly; `IronCanvas::set_theme` marks both layers dirty on change,
    /// so the overlay-only fast path never paints against a stale theme.
    pub theme: CanvasTheme,
    /// Pane content fingerprints from the *previous* frame, snapshotted
    /// in `next_frame`. Indexed by `PaneRegion as usize`. Zero on first
    /// paint and after a `Rebuild` so the natural compare always misses.
    pub(crate) prev_pane_fingerprints: [u64; 4],
    /// Pane content fingerprints written by `render_pane` after each
    /// bulk-fetch. `Cell` so paint code stays on `&Chrome` (matches the
    /// crate convention that paint never holds a mutable Chrome).
    pub(crate) pane_fingerprints: Cell<[u64; 4]>,
    /// Which constructor produced this frame. `reuses_slots()` is the
    /// migration shim for the "carry slot vecs over from prev" predicate;
    /// Stage 5 will dispatch regime arms exhaustively on this tag.
    pub(crate) kind: FrameKindTag,
    /// Which panes `render_grid` must paint this frame. `next_frame`
    /// sets this to `ALL`; Stage 3.3's `next_frame_with_blit` will
    /// narrow it when the BlitPlan proves the cross-axis panes are
    /// unchanged by the scroll.
    pub(crate) stale_panes: PaneRegionMask,
}

/// Outcome of comparing a cached `Chrome` against the live model.
///
/// Binary in Stage 1: per-pane skip happens inside `render_pane` after
/// the bulk-fetch (see `prev_pane_fingerprints`). A future `PaneSubset
/// { stale: PaneRegionMask }` variant becomes useful once bulk-fetch
/// moves into a cross-frame `PaneCache` (Stage 3 of the plan).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameValidity {
    /// Slot vecs match the live model. Caller may reuse `last_frame`
    /// directly; `render_pane` will fingerprint-skip per pane.
    SlotsReuse,
    /// Slot vecs diverged (scroll / freeze / sheet / canvas size).
    /// Caller must call `Chrome::next_frame` for a full rebuild.
    Rebuild,
}

impl Chrome {
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn pane_set_top_row_debug(&self) -> i32 {
        self.pane_set.top_row()
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn pane_set_last_row_debug(&self) -> i32 {
        self.pane_set
            .scroll_rows
            .last()
            .map(|s| s.row)
            .unwrap_or(-1)
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn scroll_rows_len_debug(&self) -> usize {
        self.pane_set.scroll_rows.len()
    }

    /// Build the next-frame `Chrome`. The `path` argument selects which
    /// regime the orchestrator chose; the body branches once and inlines
    /// the three former constructors.
    ///
    ///   * `Fresh` — full rebuild. `prev = Some` recycles slot Vec
    ///     allocations; `None` is the first-frame path. See
    ///     `ARCHITECTURE.md` for build phases A–E.
    ///   * `SlotsReuse` — prev's slot vecs survive verbatim; only
    ///     per-frame state (theme + `pane_fingerprints` rotation) is
    ///     refreshed. Caller must invoke `refresh_overlay_inputs` after.
    ///   * `Blit(plan)` — caller has already issued `Painter::blit` to
    ///     shift the kept band; this frame rebuilds only the scroll-axis
    ///     slot vec (kept band heights/widths carry over from prev; the
    ///     strip is the only band that hits the model) and clones the
    ///     cross-axis vec. Falls back to `Fresh` when `row_header_thickness`
    ///     would change across a digit boundary (e.g. row 99 → 100).
    ///     Non-stale panes get their `pane_fingerprints` seeded from prev
    ///     so the *next* frame's fingerprint compare doesn't false-
    ///     mismatch against a build-default 0.
    ///
    /// `SlotsReuse` and `Blit` require `prev = Some`; `None` falls
    /// through to `Fresh` defensively. The orchestrator proves
    /// `prev.is_some()` before selecting those paths, but the fallback
    /// keeps `Chrome::next` total.
    pub(crate) fn next(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        path: FramePath,
    ) -> Self {
        match path {
            FramePath::Fresh => {
                let (recycled, prev_fps) = match prev {
                    Some(c) => (
                        RecycledSlots::from_pane_set(c.pane_set),
                        c.pane_fingerprints.get(),
                    ),
                    None => (RecycledSlots::default(), [0u64; 4]),
                };
                Self::build(model, canvas, theme, recycled, prev_fps)
            }
            FramePath::SlotsReuse => {
                let Some(mut prev) = prev else {
                    return Self::next(None, model, canvas, theme, FramePath::Fresh);
                };
                prev.prev_pane_fingerprints = prev.pane_fingerprints.replace([0; 4]);
                prev.theme = theme.clone();
                prev.kind = FrameKindTag::SlotsReused;
                prev
            }
            FramePath::Blit(plan) => {
                let Some(prev) = prev else {
                    return Self::next(None, model, canvas, theme, FramePath::Fresh);
                };
                if let Some(frame) = blit::try_blit_reuse(&prev, model, canvas, theme, &plan) {
                    return frame;
                }
                let frame = Self::next(Some(prev), model, canvas, theme, FramePath::Fresh);
                let stale = plan.shift_panes();
                let mut fps = frame.pane_fingerprints.get();
                for region in [
                    PaneRegion::TopLeft,
                    PaneRegion::TopRight,
                    PaneRegion::BottomLeft,
                    PaneRegion::BottomRight,
                ] {
                    if !stale.contains_region(region) {
                        let idx = region as usize;
                        fps[idx] = frame.prev_pane_fingerprints[idx];
                    }
                }
                frame.pane_fingerprints.set(fps);
                Chrome {
                    kind: FrameKindTag::Blitted,
                    stale_panes: stale,
                    ..frame
                }
            }
        }
    }

    fn build(
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        recycled: RecycledSlots,
        prev_pane_fingerprints: [u64; 4],
    ) -> Self {
        // None ⇒ JS bridge transient (threw or shape malformed). Fall through
        // with the fresh-model default so the frame still builds; next animation
        // frame re-queries.
        let (top_row, left_column, selection, active_row, active_col) =
            match model.get_selected_view() {
                Some(v) => (v.top_row, v.left_column, v.selection, v.row, v.column),
                None => (
                    1,
                    1,
                    RCRange {
                        r1: 1,
                        c1: 1,
                        r2: 1,
                        c2: 1,
                    },
                    1,
                    1,
                ),
            };
        let sheet = model.get_selected_sheet();
        let active_cell = ActiveCellSnapshot::capture(model, sheet, active_row, active_col);

        // Phase A — frozen counts only.
        let frozen_row_count = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_col_count = model.get_frozen_columns_count(sheet).unwrap_or(0);

        let mut pane_set = PaneSet::with_recycled(recycled);

        // Phase B — row walk.
        let origin_y = HEADER_ROW_HEIGHT + HEADER_OFFSET;
        pane_set.fill_rows(model, frozen_row_count, origin_y, top_row, canvas.h);

        // Phase C — measure row_header_thickness from the last visible row label.
        let last_visible_row = pane_set
            .scroll_rows
            .last()
            .map(|s| s.row)
            .unwrap_or((frozen_row_count + 1).max(top_row));
        let row_header_thickness = measure_row_header_width(last_visible_row);

        // Phase D — col walk uses the measured width to anchor `origin_x`.
        let origin_x = row_header_thickness + HEADER_OFFSET;
        pane_set.fill_cols(model, frozen_col_count, origin_x, left_column, canvas.w);

        // Phase E — assemble. `cell_origin` reuses the locals from B/D so
        // there's a single source of truth for the cell-area top-left.
        let col_header_thickness = HEADER_ROW_HEIGHT;
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
            selection_range: selection,
            active_cell,
            canvas_size: canvas,
            theme: theme.clone(),
            prev_pane_fingerprints,
            pane_fingerprints: Cell::new([0; 4]),
            kind: FrameKindTag::Fresh,
            // Full repaint by default. Stage 3.3's `next_frame_with_blit`
            // will override this when scroll-blit narrows the work.
            stale_panes: PaneRegionMask::ALL,
        }
    }

    /// Verdict on the cached frame's slot-vec inputs against the live
    /// model. Per-pane content skipping happens later (inside
    /// `render_pane`) via the fingerprint compare; this method only
    /// decides whether the slot vecs themselves can be reused.
    pub(crate) fn is_still_valid(
        &self,
        model: &dyn CanvasModel,
        size: CanvasSize,
    ) -> FrameValidity {
        if size != self.canvas_size {
            return FrameValidity::Rebuild;
        }
        let Some(view) = model.get_selected_view() else {
            return FrameValidity::Rebuild;
        };
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let want_top = scroll_first(frozen_rows, view.top_row);
        let want_left = scroll_first(frozen_cols, view.left_column);
        if self.pane_set.top_row() != want_top || self.pane_set.left_column() != want_left {
            return FrameValidity::Rebuild;
        }
        if frozen_rows == self.pane_set.frozen_rows_count()
            && frozen_cols == self.pane_set.frozen_cols_count()
            && sheet == self.sheet
        {
            FrameValidity::SlotsReuse
        } else {
            FrameValidity::Rebuild
        }
    }

    /// Decide whether the live model represents a pure single-axis
    /// scroll over `self`. On success returns the geometric plan for a
    /// `Painter::blit` shift of the kept band plus the strip to repaint;
    /// on any other change (sheet, freeze, theme, canvas size, two-axis
    /// scroll, overlap row-height change, scroll past viewport) returns
    /// `None` and the caller falls through to `Chrome::next_frame` for a
    /// full rebuild.
    ///
    /// `self` is the *previous* frame's snapshot — the model arg supplies
    /// the live state to compare against.
    pub(crate) fn try_blit(
        &self,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
    ) -> Option<BlitPlan> {
        if canvas != self.canvas_size || theme != &self.theme {
            return None;
        }
        let sheet = model.get_selected_sheet();
        if sheet != self.sheet {
            return None;
        }
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        if frozen_rows != self.pane_set.frozen_rows_count()
            || frozen_cols != self.pane_set.frozen_cols_count()
        {
            return None;
        }
        let view = model.get_selected_view()?;
        let new_top = scroll_first(frozen_rows, view.top_row);
        let new_left = scroll_first(frozen_cols, view.left_column);
        let old_top = self.pane_set.top_row();
        let old_left = self.pane_set.left_column();
        // Defensive content check: if the cell we painted as active no
        // longer matches the live model, the blit's kept band would
        // shift pre-edit pixels (canonical edit-then-scroll bug when
        // consumer missed `markContentDirty`).
        if !self.active_cell.matches(model, sheet) {
            return None;
        }
        match (new_top != old_top, new_left != old_left) {
            (true, false) => blit::try_blit_rows(self, model, sheet, new_top),
            (false, true) => blit::try_blit_cols(self, model, sheet, new_left),
            // (false, false): caller already filtered no-op scrolls.
            // (true, true): two-axis scroll has no single-shift plan.
            _ => None,
        }
    }

    /// Refresh overlay-only fields (independent of the slot vecs). Call on
    /// the overlay-only fast path after `is_still_valid` returns true,
    /// and on SlotsReuse rebuilds so the active-cell snapshot tracks
    /// selection moves within an unchanged viewport.
    pub(crate) fn refresh_overlay_inputs(&mut self, model: &dyn CanvasModel) {
        if let Some(view) = model.get_selected_view() {
            self.selection_range = view.selection;
            self.active_cell =
                ActiveCellSnapshot::capture(model, self.sheet, view.row, view.column);
        }
    }

    pub(crate) fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
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
    pub(crate) fn range_rect(&self, range: RCRange) -> Option<PixelRect> {
        let p = &self.pane_set;
        let frozen_rows = p.frozen_rows_count();
        let frozen_cols = p.frozen_cols_count();

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

    pub(crate) fn autofill_handle(&self) -> Option<Point> {
        let norm = self.selection_range.normalized();
        let r2 = norm.r2;
        let c2 = norm.c2;
        if r2 >= LAST_ROW || c2 >= LAST_COLUMN {
            return None;
        }
        let p = &self.pane_set;
        if !p.row_in_frame(r2) || !p.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: p.col_to_x(c2) + p.col_extent_at(c2),
            y: p.row_to_y(r2) + p.row_extent_at(r2),
        })
    }

    pub(crate) fn autofill_handle_rect(&self) -> Option<PixelRect> {
        let p = self.autofill_handle()?;
        Some(PixelRect {
            top_left: Point {
                x: p.x - AUTOFILL_HANDLE_PX,
                y: p.y - AUTOFILL_HANDLE_PX,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        })
    }

    pub(crate) fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < self.cell_origin.x && y < self.cell_origin.y {
            return HitTest::Corner;
        }
        let p = &self.pane_set;
        if y < self.cell_origin.y {
            return match p.pixel_to_col(x) {
                Some(c) => HitTest::ColHeader(c),
                None => HitTest::Outside,
            };
        }
        if x < self.cell_origin.x {
            return match p.pixel_to_row(y) {
                Some(r) => HitTest::RowHeader(r),
                None => HitTest::Outside,
            };
        }
        let (Some(row), Some(column)) = (p.pixel_to_row(y), p.pixel_to_col(x)) else {
            return HitTest::Outside;
        };
        if let Some(h) = self.autofill_handle_rect() {
            let pad = AUTOFILL_HIT_PAD_PX;
            if x >= h.top_left.x - pad
                && x <= h.right() + pad
                && y >= h.top_left.y - pad
                && y <= h.bottom() + pad
            {
                return HitTest::AutofillHandle { row, column };
            }
        }
        HitTest::Cell { row, column }
    }

    pub(crate) fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < self.col_header_thickness && x > self.row_header_thickness {
            return self
                .pane_set
                .col_boundary_at(x, tolerance)
                .map(ResizeTarget::Column);
        }
        if x < self.row_header_thickness && y > self.col_header_thickness {
            return self
                .pane_set
                .row_boundary_at(y, tolerance)
                .map(ResizeTarget::Row);
        }
        None
    }
}
