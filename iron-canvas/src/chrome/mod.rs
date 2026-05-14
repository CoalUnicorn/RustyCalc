//! Per-frame snapshot of painted chrome geometry. The renderer and every
//! `IronCanvas` query read the same `Chrome`, so painted pixels and hit
//! zones cannot disagree.
//!
//! Pure-axis walks live on `PaneSet`; `Chrome` composes them whenever a
//! query spans both axes. See `ARCHITECTURE.md` for the build phases
//! (A–E) and the `is_still_valid` cache rules.

use std::cell::Cell;

use crate::geometry::slot::{
    boundary_at, col_width, fill_axis, last_visible_id, pixel_to_id, row_height, scroll_first,
    slot_at, top_id, AxisSlot, ColSlot, RowSlot,
};
use crate::geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_OFFSET,
        HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::{Axis, Point},
};
use crate::theme::CanvasTheme;
use crate::types::ui::{HitTest, ResizeTarget};
use crate::{CanvasModel, CanvasSize, RCRange};

pub(crate) mod pane_region;

pub(crate) use pane_region::{PaneRegion, PaneRegionMask};

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding either side of the row-label inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

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
    /// True when this Chrome was carried over from the previous frame
    /// (slot vecs identical). Tells the grid layer to skip the full
    /// canvas clear and `render_pane` that fingerprint compares against
    /// `prev_pane_fingerprints` are meaningful.
    pub(crate) slots_reused: bool,
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

/// Pure-canvas-pixel description of a scroll-blit: the kept band's source
/// and destination rects, plus the strip the renderer must repaint to
/// fill in newly-revealed content. Axis tells the orchestrator which
/// header strip to repaint (the cross-axis header is untouched by the
/// scroll).
///
/// All rects are in CSS pixels relative to the canvas origin — the
/// `Painter::blit` backend handles DPR. `src` and `dst` have identical
/// `width`/`height`; only the offset along `axis` differs.
#[derive(Clone, Copy)]
pub(crate) struct BlitPlan {
    pub axis: Axis,
    pub src: PixelRect,
    pub dst: PixelRect,
    pub repaint_strip: PixelRect,
}

impl BlitPlan {
    /// Panes whose cached pane-buffer data shifts along `axis` and which
    /// therefore need `apply_blit_shift` + strip-fetch + a repaint pass.
    /// Cross-axis panes (TopLeft on either scroll; TopRight on row scroll;
    /// BottomLeft on column scroll) are stable across the blit and are
    /// excluded.
    pub(crate) fn shift_panes(&self) -> PaneRegionMask {
        match self.axis {
            Axis::Row => PaneRegionMask::EMPTY
                .with(PaneRegion::BottomLeft)
                .with(PaneRegion::BottomRight),
            Axis::Column => PaneRegionMask::EMPTY
                .with(PaneRegion::TopRight)
                .with(PaneRegion::BottomRight),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PaneSet {
    pub frozen_rows: Vec<RowSlot>,
    pub scroll_rows: Vec<RowSlot>,
    pub frozen_offset_y: i32,
    pub frozen_cols: Vec<ColSlot>,
    pub scroll_cols: Vec<ColSlot>,
    pub frozen_offset_x: i32,
}

/// Cleared slot Vecs handed from the outgoing `Chrome` into the next one,
/// so the backing allocations cross the frame boundary and steady-state
/// rebuilds only allocate when row/column count outgrows capacity.
#[derive(Default)]
pub(crate) struct RecycledSlots {
    pub(crate) frozen_rows: Vec<RowSlot>,
    pub(crate) scroll_rows: Vec<RowSlot>,
    pub(crate) frozen_cols: Vec<ColSlot>,
    pub(crate) scroll_cols: Vec<ColSlot>,
}

impl RecycledSlots {
    fn from_pane_set(pane_set: PaneSet) -> Self {
        let PaneSet {
            mut frozen_rows,
            mut scroll_rows,
            mut frozen_cols,
            mut scroll_cols,
            ..
        } = pane_set;
        frozen_rows.clear();
        scroll_rows.clear();
        frozen_cols.clear();
        scroll_cols.clear();
        Self {
            frozen_rows,
            scroll_rows,
            frozen_cols,
            scroll_cols,
        }
    }
}

impl PaneSet {
    /// Fresh `PaneSet` reusing the previous frame's drained slot Vecs.
    /// `frozen_offset_*` are filled in by `fill_rows` / `fill_cols`.
    pub(crate) fn with_recycled(recycled: RecycledSlots) -> Self {
        PaneSet {
            frozen_rows: recycled.frozen_rows,
            scroll_rows: recycled.scroll_rows,
            frozen_offset_y: 0,
            frozen_cols: recycled.frozen_cols,
            scroll_cols: recycled.scroll_cols,
            frozen_offset_x: 0,
        }
    }

    /// Populate `frozen_rows`, `scroll_rows`, and `frozen_offset_y`
    /// (Phase B of `Chrome::build`; see `ARCHITECTURE.md`). Runs before
    /// the row-label measurement, so it does not depend on
    /// `row_header_thickness`. Reads `FROZEN_SEP` directly because the
    /// gap between frozen and scroll bands is the row axis's concern.
    pub(crate) fn fill_rows(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_y: i32,
        view_top_row: i32,
        canvas_h: f64,
    ) {
        self.frozen_rows.reserve(frozen_count as usize);
        let after_frozen = fill_axis(
            &mut self.frozen_rows,
            1..=frozen_count,
            origin_y,
            i32::MAX,
            |r| row_height(model, r),
        );
        self.frozen_offset_y = after_frozen + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let _ = fill_axis(
            &mut self.scroll_rows,
            scroll_first(frozen_count, view_top_row)..=LAST_ROW,
            self.frozen_offset_y,
            canvas_h.ceil() as i32,
            |r| row_height(model, r),
        );
    }

    /// Column-axis mirror of `fill_rows`. Runs as Phase D, using the
    /// cell-area X origin that already folds in the measured
    /// `row_header_thickness`.
    pub(crate) fn fill_cols(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_x: i32,
        view_left_column: i32,
        canvas_w: f64,
    ) {
        self.frozen_cols.reserve(frozen_count as usize);
        let after_frozen = fill_axis(
            &mut self.frozen_cols,
            1..=frozen_count,
            origin_x,
            i32::MAX,
            |c| col_width(model, c),
        );
        self.frozen_offset_x = after_frozen + if frozen_count > 0 { FROZEN_SEP } else { 0 };

        let _ = fill_axis(
            &mut self.scroll_cols,
            scroll_first(frozen_count, view_left_column)..=LAST_COLUMN,
            self.frozen_offset_x,
            canvas_w.ceil() as i32,
            |c| col_width(model, c),
        );
    }

    #[inline]
    pub(crate) fn frozen_rows_count(&self) -> i32 {
        self.frozen_rows.len() as i32
    }

    #[inline]
    pub(crate) fn frozen_cols_count(&self) -> i32 {
        self.frozen_cols.len() as i32
    }

    #[inline]
    fn row_slot(&self, row: i32) -> Option<&RowSlot> {
        slot_at(&self.frozen_rows, &self.scroll_rows, row)
    }

    #[inline]
    fn col_slot(&self, col: i32) -> Option<&ColSlot> {
        slot_at(&self.frozen_cols, &self.scroll_cols, col)
    }

    #[inline]
    pub(crate) fn top_row(&self) -> i32 {
        top_id(&self.scroll_rows)
    }

    #[inline]
    pub(crate) fn left_column(&self) -> i32 {
        top_id(&self.scroll_cols)
    }

    #[inline]
    pub(crate) fn last_visible_row(&self) -> i32 {
        last_visible_id(&self.scroll_rows)
    }

    #[inline]
    pub(crate) fn last_visible_col(&self) -> i32 {
        last_visible_id(&self.scroll_cols)
    }

    #[inline]
    pub(crate) fn row_in_frame(&self, row: i32) -> bool {
        self.row_slot(row).is_some()
    }

    #[inline]
    pub(crate) fn col_in_frame(&self, col: i32) -> bool {
        self.col_slot(col).is_some()
    }

    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.extent()).unwrap_or(0)
    }

    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.extent()).unwrap_or(0)
    }

    pub(crate) fn row_to_y(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.start()).unwrap_or(0)
    }

    pub(crate) fn col_to_x(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.start()).unwrap_or(0)
    }

    pub(crate) fn pixel_to_row(&self, y: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_rows, &self.scroll_rows, y)
    }

    pub(crate) fn pixel_to_col(&self, x: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_cols, &self.scroll_cols, x)
    }

    pub(crate) fn row_boundary_at(&self, y: i32, hit_zone: i32) -> Option<i32> {
        boundary_at(&self.frozen_rows, &self.scroll_rows, y, hit_zone)
    }

    pub(crate) fn col_boundary_at(&self, x: i32, hit_zone: i32) -> Option<i32> {
        boundary_at(&self.frozen_cols, &self.scroll_cols, x, hit_zone)
    }
}

/// Decimal digit count, clamped to `≥ 1` so a zero input still reserves a slot.
fn digit_count(n: i32) -> i32 {
    let mut n = n.max(1);
    let mut d = 0;
    while n > 0 {
        d += 1;
        n /= 10;
    }
    d
}

/// Pixel width the row-header strip needs to fit the widest visible row
/// label. Uses a pessimistic char-count approximation to avoid threading
/// `TextMetrics` (and thus a painter dependency) into `Chrome::build`.
/// Floored at `HEADER_COL_WIDTH` so 3-digit labels never shrink the strip.
pub(crate) fn measure_row_header_width(max_visible_row: i32) -> i32 {
    let digits = digit_count(max_visible_row);
    let approx = digits * APPROX_DIGIT_WIDTH_PX + 2 * HEADER_LABEL_PAD_PX;
    approx.max(HEADER_COL_WIDTH)
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

    /// Build the next-frame `Chrome`. When `prev` is `Some`, the outgoing
    /// frame's slot Vec allocations are recycled so steady-state repaints
    /// don't reallocate the four pane-set buffers. `prev == None` is the
    /// first-frame path. See `ARCHITECTURE.md` for the A–E build phases.
    pub(crate) fn next_frame(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
    ) -> Self {
        let (recycled, prev_fps) = match prev {
            Some(c) => (
                RecycledSlots::from_pane_set(c.pane_set),
                c.pane_fingerprints.get(),
            ),
            None => (RecycledSlots::default(), [0u64; 4]),
        };
        Self::build(model, canvas, theme, recycled, prev_fps)
    }

    /// Blit fast-path frame: caller has already issued the `Painter::blit`
    /// to shift the kept-band pixels into their new viewport position, so
    /// this frame inherits `slots_reused = true` and narrows `stale_panes`
    /// to just the panes whose data shifts along the scroll axis.
    ///
    /// First tries `try_blit_reuse`: rebuilds only the scroll-axis slot
    /// vec (kept band's heights/widths carry over from prev; the strip
    /// is the only band that hits the model) and clones the cross-axis
    /// slot vec verbatim — `try_blit` already verified frozen counts +
    /// canvas size are unchanged, so the cross-axis can't have shifted.
    /// Bails to the full `next_frame` walk when row_header_thickness
    /// would change (e.g. row 99 → 100 crosses a digit boundary), which
    /// is the one case where cross-axis col `.left` values shift.
    ///
    /// Non-stale panes are skipped entirely by `render_grid`, so we seed
    /// their `pane_fingerprints` from prev here — otherwise the Stage 1
    /// fingerprint compare on the *next* frame would read the build-
    /// default 0 and false-mismatch into an unnecessary repaint.
    pub(crate) fn next_frame_with_blit(
        prev: Chrome,
        model: &dyn CanvasModel,
        canvas: CanvasSize,
        theme: &CanvasTheme,
        plan: &BlitPlan,
    ) -> Self {
        if let Some(frame) = try_blit_reuse(&prev, model, canvas, theme, plan) {
            return frame;
        }
        let frame = Self::next_frame(Some(prev), model, canvas, theme);
        let stale = plan.shift_panes();
        let mut fps = frame.pane_fingerprints.get();
        for region in [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ] {
            if !stale.contains(region) {
                let idx = region as usize;
                fps[idx] = frame.prev_pane_fingerprints[idx];
            }
        }
        frame.pane_fingerprints.set(fps);
        Chrome {
            slots_reused: true,
            stale_panes: stale,
            ..frame
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
        let (top_row, left_column, selection) = match model.get_selected_view() {
            Some(v) => (v.top_row, v.left_column, v.selection),
            None => (
                1,
                1,
                RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 1,
                    c2: 1,
                },
            ),
        };
        let sheet = model.get_selected_sheet();

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
            canvas_size: canvas,
            theme: theme.clone(),
            prev_pane_fingerprints,
            pane_fingerprints: Cell::new([0; 4]),
            slots_reused: false,
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
        match (new_top != old_top, new_left != old_left) {
            (true, false) => try_blit_rows(self, model, sheet, new_top),
            (false, true) => try_blit_cols(self, model, sheet, new_left),
            // (false, false): caller already filtered no-op scrolls.
            // (true, true): two-axis scroll has no single-shift plan.
            _ => None,
        }
    }

    /// Refresh overlay-only fields (independent of the slot vecs). Call on
    /// the overlay-only fast path after `is_still_valid` returns true.
    pub(crate) fn refresh_overlay_inputs(&mut self, model: &dyn CanvasModel) {
        if let Some(view) = model.get_selected_view() {
            self.selection_range = view.selection;
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

// ─── Scroll-blit helpers ──────────────────────────────────────────────────
//
// `try_blit` already disqualified anything that isn't a pure single-axis
// scroll. These helpers compute the canvas-pixel src/dst/strip rects and
// verify the kept band's row heights (col widths) match what the model
// still reports — that is the final qualification that the shifted pixels
// will land where the new chrome would paint them.

/// Build the next-frame Chrome by reusing as much of `prev` as the blit
/// plan guarantees is unchanged: cross-axis slot vec is cloned verbatim,
/// scroll-axis kept band carries forward heights/widths, only the strip
/// touches the model. Returns `None` on the one cross-axis-affecting
/// edge case (row_header_thickness changes across a digit boundary) or
/// any model anomaly — the caller falls through to a full `next_frame`.
fn try_blit_reuse(
    prev: &Chrome,
    model: &dyn CanvasModel,
    canvas: CanvasSize,
    theme: &CanvasTheme,
    plan: &BlitPlan,
) -> Option<Chrome> {
    let view = model.get_selected_view()?;
    let frozen_rows_count = prev.pane_set.frozen_rows_count();
    let frozen_cols_count = prev.pane_set.frozen_cols_count();
    let new_top = scroll_first(frozen_rows_count, view.top_row);
    let new_left = scroll_first(frozen_cols_count, view.left_column);

    let (scroll_rows, scroll_cols) = match plan.axis {
        Axis::Row => (
            rebuild_rows_for_row_scroll(prev, model, new_top, canvas)?,
            prev.pane_set.scroll_cols.clone(),
        ),
        Axis::Column => (
            prev.pane_set.scroll_rows.clone(),
            rebuild_cols_for_col_scroll(prev, model, new_left, canvas)?,
        ),
    };

    // Row header thickness gates cross-axis reuse. If the new last
    // visible row label grew (e.g. row 99 → 100), origin_x shifts and
    // every col slot's `.left` is off — fall back to full rebuild.
    let last_visible_row = scroll_rows
        .last()
        .map(|s| s.row)
        .unwrap_or((frozen_rows_count + 1).max(new_top));
    let row_header_thickness = measure_row_header_width(last_visible_row);
    if row_header_thickness != prev.row_header_thickness {
        return None;
    }

    let pane_set = PaneSet {
        frozen_rows: prev.pane_set.frozen_rows.clone(),
        scroll_rows,
        frozen_offset_y: prev.pane_set.frozen_offset_y,
        frozen_cols: prev.pane_set.frozen_cols.clone(),
        scroll_cols,
        frozen_offset_x: prev.pane_set.frozen_offset_x,
    };

    let stale = plan.shift_panes();
    let prev_fps = prev.pane_fingerprints.get();
    let mut seeded_fps = [0u64; 4];
    for region in [
        PaneRegion::TopLeft,
        PaneRegion::TopRight,
        PaneRegion::BottomLeft,
        PaneRegion::BottomRight,
    ] {
        if !stale.contains(region) {
            let idx = region as usize;
            seeded_fps[idx] = prev_fps[idx];
        }
    }

    Some(Chrome {
        sheet: prev.sheet,
        pane_set,
        row_header_thickness,
        col_header_thickness: prev.col_header_thickness,
        cell_origin: prev.cell_origin,
        selection_range: view.selection,
        canvas_size: canvas,
        theme: theme.clone(),
        prev_pane_fingerprints: prev_fps,
        pane_fingerprints: Cell::new(seeded_fps),
        slots_reused: true,
        stale_panes: stale,
    })
}

/// Build new `scroll_rows` for a pure row scroll: keep the surviving
/// slots with their `.row` and `.height` intact and only `.top` shifted,
/// then `fill_axis` the strip from the model. Returns `None` if `prev`'s
/// data isn't enough to cover the kept band — try_blit guards make this
/// path unreachable, but cheap defensiveness keeps the fallback open.
fn rebuild_rows_for_row_scroll(
    prev: &Chrome,
    model: &dyn CanvasModel,
    new_top: i32,
    canvas: CanvasSize,
) -> Option<Vec<RowSlot>> {
    let prev_rows = &prev.pane_set.scroll_rows;
    let frozen_offset_y = prev.pane_set.frozen_offset_y;
    let max_cursor = canvas.h.ceil() as i32;
    let delta = new_top - prev.pane_set.top_row();
    if delta == 0 {
        return None;
    }
    let drows = delta.unsigned_abs() as usize;
    if drows >= prev_rows.len() {
        return None;
    }

    let mut new_rows: Vec<RowSlot> = Vec::with_capacity(prev_rows.len() + drows);

    if delta > 0 {
        // Scroll DOWN — drop leading `drows` rows; strip is appended below
        // by the topup fill_axis.
        let leaving_h = prev_rows[drows].top - frozen_offset_y;
        for slot in &prev_rows[drows..] {
            new_rows.push(RowSlot {
                row: slot.row,
                top: slot.top - leaving_h,
                height: slot.height,
            });
        }
    } else {
        // Scroll UP — strip enters at top, kept band shifts down by strip_h.
        let strip_last = prev_rows[0].row - 1;
        if new_top > strip_last {
            return None;
        }
        let strip_cursor_end = fill_axis(
            &mut new_rows,
            new_top..=strip_last,
            frozen_offset_y,
            i32::MAX,
            |r| row_height(model, r),
        );
        let strip_h = strip_cursor_end - frozen_offset_y;
        for slot in &prev_rows[..prev_rows.len() - drows] {
            new_rows.push(RowSlot {
                row: slot.row,
                top: slot.top + strip_h,
                height: slot.height,
            });
        }
    }

    // Slot vec invariant (matches `fill_axis`): at most one row may have
    // `top >= max_cursor` — the overflow row included for partial-edge
    // rendering. After the shift, the inherited overflow plus newly-
    // pushed ones can leave two overflow rows; trim back to one. Then,
    // if no overflow row exists yet, fill_axis pushes exactly one.
    while new_rows.len() >= 2
        && new_rows[new_rows.len() - 1].top >= max_cursor
        && new_rows[new_rows.len() - 2].top >= max_cursor
    {
        new_rows.pop();
    }
    if new_rows.last().is_some_and(|s| s.top < max_cursor) {
        let cursor = new_rows
            .last()
            .map(|s| s.top + s.height)
            .unwrap_or(frozen_offset_y);
        let next_row = new_rows.last().map(|s| s.row + 1).unwrap_or(new_top);
        let _ = fill_axis(
            &mut new_rows,
            next_row..=LAST_ROW,
            cursor,
            max_cursor,
            |r| row_height(model, r),
        );
    }

    Some(new_rows)
}

/// Column-scroll mirror of `rebuild_rows_for_row_scroll`.
fn rebuild_cols_for_col_scroll(
    prev: &Chrome,
    model: &dyn CanvasModel,
    new_left: i32,
    canvas: CanvasSize,
) -> Option<Vec<ColSlot>> {
    let prev_cols = &prev.pane_set.scroll_cols;
    let frozen_offset_x = prev.pane_set.frozen_offset_x;
    let max_cursor = canvas.w.ceil() as i32;
    let delta = new_left - prev.pane_set.left_column();
    if delta == 0 {
        return None;
    }
    let dcols = delta.unsigned_abs() as usize;
    if dcols >= prev_cols.len() {
        return None;
    }

    let mut new_cols: Vec<ColSlot> = Vec::with_capacity(prev_cols.len() + dcols);

    if delta > 0 {
        let leaving_w = prev_cols[dcols].left - frozen_offset_x;
        for slot in &prev_cols[dcols..] {
            new_cols.push(ColSlot {
                col: slot.col,
                left: slot.left - leaving_w,
                width: slot.width,
            });
        }
    } else {
        let strip_last = prev_cols[0].col - 1;
        if new_left > strip_last {
            return None;
        }
        let strip_cursor_end = fill_axis(
            &mut new_cols,
            new_left..=strip_last,
            frozen_offset_x,
            i32::MAX,
            |c| col_width(model, c),
        );
        let strip_w = strip_cursor_end - frozen_offset_x;
        for slot in &prev_cols[..prev_cols.len() - dcols] {
            new_cols.push(ColSlot {
                col: slot.col,
                left: slot.left + strip_w,
                width: slot.width,
            });
        }
    }

    while new_cols.len() >= 2
        && new_cols[new_cols.len() - 1].left >= max_cursor
        && new_cols[new_cols.len() - 2].left >= max_cursor
    {
        new_cols.pop();
    }
    if new_cols.last().is_some_and(|s| s.left < max_cursor) {
        let cursor = new_cols
            .last()
            .map(|s| s.left + s.width)
            .unwrap_or(frozen_offset_x);
        let next_col = new_cols.last().map(|s| s.col + 1).unwrap_or(new_left);
        let _ = fill_axis(
            &mut new_cols,
            next_col..=LAST_COLUMN,
            cursor,
            max_cursor,
            |c| col_width(model, c),
        );
    }

    Some(new_cols)
}

fn try_blit_rows(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_top: i32,
) -> Option<BlitPlan> {
    let prev_rows = &prev.pane_set.scroll_rows;
    let _last = prev_rows.last()?;
    let pane_x = prev.pane_set.frozen_offset_x;
    let pane_y = prev.pane_set.frozen_offset_y;
    // pane_h is bounded by the canvas backing store extent, not by
    // `scroll_rows.last().top + height`. `fill_axis` pushes one row past
    // the canvas edge (the "overflow row") whose pixels were never on
    // canvas — using slot-bound pane_h here would send drawImage's
    // source rect past the backing store and the spec's proportional
    // source/dest clip would leave the bottom row stale.
    let pane_w = (prev.canvas_size.w.round() as i32) - pane_x;
    let pane_h = (prev.canvas_size.h.round() as i32) - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let old_top = prev.pane_set.top_row();

    if new_top > old_top {
        // Scroll DOWN — leaving rows are prev.scroll_rows[0..drows].
        let drows = (new_top - old_top) as usize;
        if drows >= prev_rows.len() {
            return None;
        }
        let leaving_h = prev_rows[drows].top - pane_y;
        if leaving_h <= 0 || leaving_h >= pane_h {
            return None;
        }
        if !overlap_row_heights_match(model, sheet, &prev_rows[drows..]) {
            return None;
        }
        Some(plan_along_y(
            Axis::Row,
            pane_x,
            pane_w,
            pane_y,
            pane_h,
            leaving_h,
            ShiftDir::Up,
            prev.canvas_size.h.round() as i32,
        ))
    } else {
        // Scroll UP — the new top rows aren't in `prev`; query the model
        // for their heights to compute |Δpx|.
        let drows = (old_top - new_top) as usize;
        let mut strip_h: i32 = 0;
        for i in 0..drows {
            let h = model
                .get_row_height(sheet, new_top + i as i32)
                .unwrap_or(0.0)
                .round() as i32;
            strip_h = strip_h.saturating_add(h);
        }
        if strip_h <= 0 || strip_h >= pane_h {
            return None;
        }
        if !overlap_row_heights_match(model, sheet, prev_rows) {
            return None;
        }
        Some(plan_along_y(
            Axis::Row,
            pane_x,
            pane_w,
            pane_y,
            pane_h,
            strip_h,
            ShiftDir::Down,
            prev.canvas_size.h.round() as i32,
        ))
    }
}

fn try_blit_cols(
    prev: &Chrome,
    model: &dyn CanvasModel,
    sheet: u32,
    new_left: i32,
) -> Option<BlitPlan> {
    let prev_cols = &prev.pane_set.scroll_cols;
    let _last = prev_cols.last()?;
    let pane_x = prev.pane_set.frozen_offset_x;
    let pane_y = prev.pane_set.frozen_offset_y;
    // pane_w is bounded by the canvas backing store extent, not by
    // `scroll_cols.last().left + width` — `fill_axis` pushes one column
    // past the canvas edge (the "overflow column") whose pixels were
    // never on canvas. Mirror of try_blit_rows. See the comment there.
    let pane_w = (prev.canvas_size.w.round() as i32) - pane_x;
    let pane_h = (prev.canvas_size.h.round() as i32) - pane_y;
    if pane_w <= 0 || pane_h <= 0 {
        return None;
    }
    let old_left = prev.pane_set.left_column();

    if new_left > old_left {
        let dcols = (new_left - old_left) as usize;
        if dcols >= prev_cols.len() {
            return None;
        }
        let leaving_w = prev_cols[dcols].left - pane_x;
        if leaving_w <= 0 || leaving_w >= pane_w {
            return None;
        }
        if !overlap_col_widths_match(model, sheet, &prev_cols[dcols..]) {
            return None;
        }
        Some(plan_along_x(
            Axis::Column,
            pane_y,
            pane_h,
            pane_x,
            pane_w,
            leaving_w,
            ShiftDir::Up,
            prev.canvas_size.w.round() as i32,
        ))
    } else {
        let dcols = (old_left - new_left) as usize;
        let mut strip_w: i32 = 0;
        for i in 0..dcols {
            let w = model
                .get_column_width(sheet, new_left + i as i32)
                .unwrap_or(0.0)
                .round() as i32;
            strip_w = strip_w.saturating_add(w);
        }
        if strip_w <= 0 || strip_w >= pane_w {
            return None;
        }
        if !overlap_col_widths_match(model, sheet, prev_cols) {
            return None;
        }
        Some(plan_along_x(
            Axis::Column,
            pane_y,
            pane_h,
            pane_x,
            pane_w,
            strip_w,
            ShiftDir::Down,
            prev.canvas_size.w.round() as i32,
        ))
    }
}

fn overlap_row_heights_match(model: &dyn CanvasModel, sheet: u32, overlap: &[RowSlot]) -> bool {
    overlap.iter().all(|s| {
        model
            .get_row_height(sheet, s.row)
            .map(|h| h.round() as i32 == s.height)
            .unwrap_or(false)
    })
}

fn overlap_col_widths_match(model: &dyn CanvasModel, sheet: u32, overlap: &[ColSlot]) -> bool {
    overlap.iter().all(|s| {
        model
            .get_column_width(sheet, s.col)
            .map(|w| w.round() as i32 == s.width)
            .unwrap_or(false)
    })
}

#[derive(Copy, Clone)]
enum ShiftDir {
    /// Content moves toward smaller coordinate; strip lands at the far edge
    /// (scroll DOWN on rows, scroll RIGHT on cols).
    Up,
    /// Content moves toward larger coordinate; strip lands at the near edge
    /// (scroll UP on rows, scroll LEFT on cols).
    Down,
}

/// Compose a row-axis `BlitPlan`. `pane_h` is the prev frame's painted
/// scrollable height; `shift_px` is the absolute pixel shift. `canvas_h`
/// caps the repaint strip so it covers the entire untouched zone below
/// the previous paint, in case `pane_h` was short of the canvas edge.
#[allow(clippy::too_many_arguments)]
fn plan_along_y(
    axis: Axis,
    pane_x: i32,
    pane_w: i32,
    pane_y: i32,
    pane_h: i32,
    shift_px: i32,
    dir: ShiftDir,
    canvas_h: i32,
) -> BlitPlan {
    let kept_h = pane_h - shift_px;
    let (src_y, dst_y, strip_y, strip_h) = match dir {
        ShiftDir::Up => {
            // Source is below the leaving band, dest sits at pane top.
            // Repaint strip covers everything below the shifted band,
            // through the canvas edge.
            let strip_y = pane_y + kept_h;
            (
                pane_y + shift_px,
                pane_y,
                strip_y,
                (canvas_h - strip_y).max(shift_px),
            )
        }
        ShiftDir::Down => {
            // Source at pane top, dest shifted down. Strip fills the
            // newly-revealed top band.
            (pane_y, pane_y + shift_px, pane_y, shift_px)
        }
    };
    BlitPlan {
        axis,
        src: PixelRect {
            top_left: Point {
                x: pane_x,
                y: src_y,
            },
            width: pane_w,
            height: kept_h,
        },
        dst: PixelRect {
            top_left: Point {
                x: pane_x,
                y: dst_y,
            },
            width: pane_w,
            height: kept_h,
        },
        repaint_strip: PixelRect {
            top_left: Point {
                x: pane_x,
                y: strip_y,
            },
            width: pane_w,
            height: strip_h,
        },
    }
}

/// Column-axis mirror of `plan_along_y`. `canvas_w` caps the strip in
/// the scroll-RIGHT case where the previous paint didn't reach the right
/// edge of the canvas.
#[allow(clippy::too_many_arguments)]
fn plan_along_x(
    axis: Axis,
    pane_y: i32,
    pane_h: i32,
    pane_x: i32,
    pane_w: i32,
    shift_px: i32,
    dir: ShiftDir,
    canvas_w: i32,
) -> BlitPlan {
    let kept_w = pane_w - shift_px;
    let (src_x, dst_x, strip_x, strip_w) = match dir {
        ShiftDir::Up => {
            let strip_x = pane_x + kept_w;
            (
                pane_x + shift_px,
                pane_x,
                strip_x,
                (canvas_w - strip_x).max(shift_px),
            )
        }
        ShiftDir::Down => (pane_x, pane_x + shift_px, pane_x, shift_px),
    };
    BlitPlan {
        axis,
        src: PixelRect {
            top_left: Point {
                x: src_x,
                y: pane_y,
            },
            width: kept_w,
            height: pane_h,
        },
        dst: PixelRect {
            top_left: Point {
                x: dst_x,
                y: pane_y,
            },
            width: kept_w,
            height: pane_h,
        },
        repaint_strip: PixelRect {
            top_left: Point {
                x: strip_x,
                y: pane_y,
            },
            width: strip_w,
            height: pane_h,
        },
    }
}
