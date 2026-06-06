//! Per-frame snapshot of painted chrome geometry. The renderer and every
//! `Orchestrator` query read the same `Chrome`, so painted pixels and hit
//! zones cannot disagree.
//!
//! Pure-axis walks live on `PaneSet`; `Chrome` composes them whenever a
//! query spans both axes. See `ARCHITECTURE.md` for the build phases
//! (A–E) and the `is_still_valid` cache rules.

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::geometry::{
    constants::{AUTOFILL_HANDLE_PX, CELL_AREA_INSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW},
    pixel_rect::PixelRect,
    prim::Point,
    slot::scroll_first,
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, CanvasView, RCRange};

mod blit;
mod blit_rebuild;
mod kind;
mod pane_region;
mod pane_set;
mod recycled_slots;

pub use blit::{BlitPlan, FramePath};
pub use kind::FrameKindTag;
pub use pane_region::{PaneRegion, PaneRegionMask};
pub use pane_set::{PaneSet, measure_row_header_width};
pub use recycled_slots::RecycledSlots;

/// Per-process digest of a formatted cell value. `DefaultHasher` is
/// randomly seeded per run, so equality only holds within one process.
/// The newtype shape blocks accidental serialization / cross-process
/// comparison at the type system level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellValueHash(u64);

/// Snapshot of the active cell at paint time. `screen_for_blit` re-hashes the
/// live model's value at the stored coords; a mismatch means the cell
/// was edited since this `Chrome` was painted, and the blit's kept band
/// would carry stale pixels. Catches the canonical edit-then-scroll case
/// without requiring the consumer to call `markContentDirty`.
#[derive(Clone, Debug, Default)]
pub struct ActiveCellSnapshot {
    pub row: i32,
    pub col: i32,
    pub value_hash: CellValueHash,
}

fn hash_cell_value(model: &dyn CanvasModel, sheet: u32, row: i32, col: i32) -> CellValueHash {
    let value = model
        .get_formatted_cell_value(sheet, row, col)
        .unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    CellValueHash(hasher.finish())
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
        hash_cell_value(model, sheet, self.row, self.col) == self.value_hash
    }
}

#[derive(Debug)]
pub struct Chrome {
    pub sheet: u32,
    pub pane_set: PaneSet,
    /// Measured per frame from the widest visible row label.
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    /// Top-left of the cell area; single source of truth for hit-test
    /// and viewport math.
    pub cell_origin: Point,
    /// Canvas size at build time. `is_still_valid` reads this to detect
    /// a resize.
    pub canvas_size: CanvasSize,
    /// Theme this frame was painted with. The renderer reads `frame.theme`
    /// directly; `IronCanvas::set_theme` marks both layers dirty on change,
    /// so the overlay-only fast path never paints against a stale theme.
    pub theme: CanvasTheme,
    /// Pane content fingerprints from the *previous* frame, snapshotted
    /// in `Chrome::next`. Indexed by `PaneRegion as usize`. Zero on first
    /// paint and after a `Rebuild` so the natural compare always misses.
    pub prev_pane_fingerprints: [u64; 4],
    /// Pane content fingerprints written by `render_pane` after each
    /// bulk-fetch. `Cell` so paint code stays on `&Chrome` (matches the
    /// crate convention that paint never holds a mutable Chrome).
    pub pane_fingerprints: Cell<[u64; 4]>,
    /// Which constructor produced this frame. Renderer diagnostics and
    /// paint-skip gating read it; `FrameKindTag::reuses_slots()` is the
    /// "slot vecs inherited from prev" predicate.
    pub kind: FrameKindTag,
    /// Which panes `render_grid` must paint this frame. `FramePath::Fresh`
    /// sets this to `ALL`; `FramePath::Blit` narrows it to the panes the
    /// `BlitPlan` shifts (cross-axis panes left intact are excluded);
    /// `FramePath::SlotsReuse { stale_panes }` takes it from the caller so
    /// it never inherits a prior `Blit` frame's narrow mask.
    pub stale_panes: PaneRegionMask,
}

/// Outcome of comparing a cached `Chrome` against the live model.
/// Per-pane skipping happens later inside `render_pane` via the
/// fingerprint compare; this verdict only gates slot-vec reuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "FrameValidity gates slot-vec reuse; dropping the verdict will force a wrong dispatch later"]
pub enum FrameValidity {
    /// Slot vecs match the live model. Caller may reuse `last_frame`
    /// directly; `render_pane` will fingerprint-skip per pane.
    SlotsReuse,
    /// Slot vecs diverged (scroll / freeze / sheet / canvas size).
    /// Caller must call `Chrome::next` with `FramePath::Fresh` for a
    /// full rebuild.
    Rebuild,
}

impl Chrome {
    /// Build the next-frame `Chrome`. The `path` argument selects which
    /// regime the orchestrator chose; the body branches once and inlines
    /// the three former constructors.
    ///
    ///   * `Fresh` — full rebuild. `prev = Some` recycles slot Vec
    ///     allocations; `None` is the first-frame path. See
    ///     `ARCHITECTURE.md` for build phases A–E.
    ///   * `SlotsReuse` — prev's slot vecs survive verbatim; only
    ///     per-frame state (theme + `pane_fingerprints` rotation) is
    ///     refreshed. Caller refreshes overlay state separately
    ///     (`SelectionLayer::refresh` in the orchestrator).
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
    pub fn next(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        path: FramePath<'_>,
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
            FramePath::SlotsReuse { stale_panes } => {
                let Some(mut prev) = prev else {
                    return Self::next(None, model, canvas, theme, FramePath::Fresh);
                };
                prev.prev_pane_fingerprints = prev.pane_fingerprints.replace([0; 4]);
                prev.theme = theme.clone();
                prev.kind = FrameKindTag::SlotsReused;
                prev.stale_panes = stale_panes;
                prev
            }
            FramePath::Blit(plan) => {
                let Some(prev) = prev else {
                    return Self::next(None, model, canvas, theme, FramePath::Fresh);
                };
                if let Some(frame) = blit::try_blit_reuse(&prev, model, canvas, theme, plan) {
                    return frame;
                }
                // Qualification passed (`screen_for_blit` returned a plan) but
                // in-place reuse rejected — e.g. row-header digit boundary at
                // 99 → 100, where `row_header_thickness` widens by one digit
                // and the cross-axis cell-area origin shifts. The frame
                // returns as `Fresh` (not the `Blitted` mislabel of yore);
                // `paint_viewport_regime` dispatches on `frame.kind` and
                // calls the full `paint_grid` path with cache invalidation.
                Self::next(Some(prev), model, canvas, theme, FramePath::Fresh)
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
        let view = model.get_selected_view().unwrap_or(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            },
            top_row: 1,
            left_column: 1,
        });
        let sheet = model.get_selected_sheet();

        // Visibility is modelled as thickness 0. CELL_AREA_INSET only reserves
        // the 1-px chrome border that draw_corner_box strokes; a hidden strip
        // paints no such border, so its thickness AND inset collapse to 0 and
        // cells reclaim the full edge (cell_origin follows from origin_x/_y).
        let show_row = model.get_show_row_headers(sheet).unwrap_or(true);
        let show_col = model.get_show_col_headers(sheet).unwrap_or(true);

        // Phase A — frozen counts only.
        let frozen_row_count = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_col_count = model.get_frozen_columns_count(sheet).unwrap_or(0);

        let mut pane_set = PaneSet::with_recycled(recycled);

        // Phase B — row walk.
        let origin_y = if show_col {
            HEADER_ROW_HEIGHT + CELL_AREA_INSET
        } else {
            0
        };
        pane_set.fill_rows(model, frozen_row_count, origin_y, view.top_row, canvas.h);

        // Phase C — measure row_header_thickness from the last visible row label.
        let last_visible_row = pane_set
            .scroll_rows
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
            frozen_col_count,
            origin_x,
            view.left_column,
            canvas.w,
        );

        // Data-driven header labels in walk_header_strip (frozen ++ scroll)
        // order so header_strip can zip slots <-> labels positionally.
        pane_set.row_header_labels =
            PaneSet::resolve_row_labels(model, sheet, &pane_set.frozen_rows, &pane_set.scroll_rows);
        pane_set.col_header_labels =
            PaneSet::resolve_col_labels(model, sheet, &pane_set.frozen_cols, &pane_set.scroll_cols);

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
            theme: theme.clone(),
            prev_pane_fingerprints,
            pane_fingerprints: Cell::new([0; 4]),
            kind: FrameKindTag::Fresh,
            // Full repaint by default. The `FramePath::Blit` arm of
            // `Chrome::next` overrides this when scroll-blit narrows the work.
            stale_panes: PaneRegionMask::ALL,
        }
    }

    /// Verdict on the cached frame's slot-vec inputs against the live
    /// model. Per-pane content skipping happens later (inside
    /// `render_pane`) via the fingerprint compare; this method only
    /// decides whether the slot vecs themselves can be reused.
    pub fn is_still_valid(&self, model: &dyn CanvasModel, size: CanvasSize) -> FrameValidity {
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
    /// `None` and the caller falls through to `Chrome::next` with
    /// `FramePath::Fresh` for a full rebuild.
    ///
    /// `self` is the *previous* frame's snapshot — the model arg supplies
    /// the live state to compare against.
    pub fn screen_for_blit(
        &self,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        active_cell: &ActiveCellSnapshot,
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
        // consumer missed `markContentDirty`). Snapshot is sourced from
        // `SelectionLayer` by the orchestrator.
        if !active_cell.matches(model, sheet) {
            return None;
        }
        match (new_top != old_top, new_left != old_left) {
            (true, false) => blit::try_blit_rows(self, model, sheet, new_top),
            (false, true) => blit::try_blit_cols(self, model, sheet, new_left),
            // (false, false): caller already filtered no-op scrolls.
            // (true, true): two-axis scroll has no single-shift plan.
            (false, false) | (true, true) => None,
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

    pub fn autofill_handle(&self, selection_range: RCRange) -> Option<Point> {
        let norm = selection_range.normalized();
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
            return match p.pixel_to_col(x) {
                Some(c) => HitTest::ColumnHeader(c),
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
        // `AutofillHandle` is resolved by `AutofillLayer::hit_test` in
        // the orchestrator's reverse-z walk; the grid path returns plain
        // cell / header / corner / outside.
        HitTest::Cell { row, column }
    }

    pub fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < self.col_header_thickness && x > self.row_header_thickness {
            return self
                .pane_set
                .col_boundary_at(x, tolerance)
                .map(ResizeTarget::ColumnEdge);
        }
        if x < self.row_header_thickness && y > self.col_header_thickness {
            return self
                .pane_set
                .row_boundary_at(y, tolerance)
                .map(ResizeTarget::RowEdge);
        }
        None
    }
}
