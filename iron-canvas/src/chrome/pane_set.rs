//! Pure-axis machinery: per-axis slot walks, frozen/scroll partitioning,
//! and the row-header width measurement that anchors the cell-area origin.
//!
//! `Chrome` composes these axis-symmetric methods whenever a query spans
//! both axes; everything here knows about only one axis at a time.

use crate::geometry::constants::{FROZEN_SEP, HEADER_COL_WIDTH, LAST_COLUMN, LAST_ROW};
use crate::geometry::slot::{
    boundary_at, col_width, fill_axis, last_visible_id, pixel_to_id, row_height, scroll_first,
    slot_at, top_id, AxisSlot, ColSlot, RowSlot,
};
use crate::{CanvasModel, CanvasSize};

/// Direction of a single-axis viewport shift between `prev.pane_set.top_row()`
/// (or `left_column()`) and the new effective scroll start.
///
/// `Forward`: new > old; kept band moves toward smaller coordinate, strip
/// lands at the far edge of the pane.
/// `Backward`: new < old; kept band moves toward larger coordinate, strip
/// lands at the near edge.
#[derive(Copy, Clone)]
pub(crate) enum ShiftDir {
    Forward,
    Backward,
}

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding either side of the row-label inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

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
    pub(super) fn from_pane_set(pane_set: PaneSet) -> Self {
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

    /// Verify that every slot's extent still matches what the model reports.
    /// `measure(id)` returns `None` when the model has no data for that id;
    /// `None` rejects the match (the kept band would no longer survive the blit).
    pub(super) fn overlaps_match<S: AxisSlot>(
        slots: &[S],
        measure: impl Fn(i32) -> Option<i32>,
    ) -> bool {
        slots.iter().all(|s| measure(s.id()) == Some(s.extent()))
    }

    /// Generic single-axis blit-probe. Returns `Some((shift_px, dir))` when
    /// the kept band would still match the model after shifting from
    /// `prev_slots[0].id()` to `new_first_idx`; `None` when geometry,
    /// extent, or overlap rejects the shift. `pane_origin` / `pane_extent`
    /// are the scroll pane's canvas-pixel bounds along the scroll axis.
    /// `measure(id)` is the model accessor; `None` returns reject overlap
    /// (rebuild treats `None` as zero, but probe rejects).
    fn probe_axis_shift<S: AxisSlot>(
        prev_slots: &[S],
        new_first_idx: i32,
        pane_origin: i32,
        pane_extent: i32,
        measure: impl Fn(i32) -> Option<i32>,
    ) -> Option<(i32, ShiftDir)> {
        let first = prev_slots.first()?;
        let old_first_idx = first.id();
        if new_first_idx > old_first_idx {
            let d = (new_first_idx - old_first_idx) as usize;
            if d >= prev_slots.len() {
                return None;
            }
            let leaving = prev_slots[d].start() - pane_origin;
            if leaving <= 0 || leaving >= pane_extent {
                return None;
            }
            if !Self::overlaps_match(&prev_slots[d..], &measure) {
                return None;
            }
            Some((leaving, ShiftDir::Forward))
        } else {
            let d = (old_first_idx - new_first_idx) as usize;
            let strip: i32 = (0..d)
                .map(|i| measure(new_first_idx + i as i32).unwrap_or(0))
                .fold(0, i32::saturating_add);
            if strip <= 0 || strip >= pane_extent {
                return None;
            }
            if !Self::overlaps_match(prev_slots, &measure) {
                return None;
            }
            Some((strip, ShiftDir::Backward))
        }
    }

    pub(crate) fn probe_row_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_top: i32,
        pane_y: i32,
        pane_h: i32,
    ) -> Option<(i32, ShiftDir)> {
        Self::probe_axis_shift(&self.scroll_rows, new_top, pane_y, pane_h, |r| {
            model.get_row_height(sheet, r).map(|h| h.round() as i32)
        })
    }

    pub(crate) fn probe_col_shift(
        &self,
        model: &dyn CanvasModel,
        sheet: u32,
        new_left: i32,
        pane_x: i32,
        pane_w: i32,
    ) -> Option<(i32, ShiftDir)> {
        Self::probe_axis_shift(&self.scroll_cols, new_left, pane_x, pane_w, |c| {
            model.get_column_width(sheet, c).map(|w| w.round() as i32)
        })
    }

    /// Rebuild a scroll-axis slot vec for a single-axis blit. The kept
    /// band's slots carry forward verbatim with only their `start()`
    /// shifted; the strip (newly-revealed band) is walked from the model
    /// via `fill_axis`. Trims trailing overflow slots so at most one slot
    /// straddles `max_cursor` (matches `fill_axis`'s invariant), then
    /// tops up if the kept-band shift left the last slot inside the
    /// canvas edge.
    ///
    /// Returns `None` when `prev_slots` is empty, `delta == 0`, the shift
    /// exceeds the kept-band capacity, or the backward strip would extend
    /// past the previous first slot.
    fn rebuild_axis_slots<S: AxisSlot>(
        prev_slots: &[S],
        frozen_offset: i32,
        max_cursor: i32,
        new_first_idx: i32,
        last_idx_limit: i32,
        measure: impl Fn(i32) -> i32,
    ) -> Option<Vec<S>> {
        let first = prev_slots.first()?;
        let old_first_idx = first.id();
        let delta = new_first_idx - old_first_idx;
        if delta == 0 {
            return None;
        }
        let d = delta.unsigned_abs() as usize;
        if d >= prev_slots.len() {
            return None;
        }

        let mut new_slots: Vec<S> = Vec::with_capacity(prev_slots.len() + d);

        if delta > 0 {
            // Forward: drop leading d slots; top up at the far edge below.
            let leaving = prev_slots[d].start() - frozen_offset;
            for slot in &prev_slots[d..] {
                new_slots.push(S::new(slot.id(), slot.start() - leaving, slot.extent()));
            }
        } else {
            // Backward: strip enters at the near edge; kept band shifts by strip_size.
            let strip_last = old_first_idx - 1;
            if new_first_idx > strip_last {
                return None;
            }
            let strip_cursor_end = fill_axis(
                &mut new_slots,
                new_first_idx..=strip_last,
                frozen_offset,
                i32::MAX,
                &measure,
            );
            let strip_size = strip_cursor_end - frozen_offset;
            for slot in &prev_slots[..prev_slots.len() - d] {
                new_slots.push(S::new(slot.id(), slot.start() + strip_size, slot.extent()));
            }
        }

        // Trim back to at most one overflow slot past max_cursor.
        while new_slots.len() >= 2
            && new_slots[new_slots.len() - 1].start() >= max_cursor
            && new_slots[new_slots.len() - 2].start() >= max_cursor
        {
            new_slots.pop();
        }
        // Top up if the kept-band shift left the last slot short of the edge.
        if new_slots.last().is_some_and(|s| s.start() < max_cursor) {
            let cursor = new_slots.last().map(|s| s.end()).unwrap_or(frozen_offset);
            let next_id = new_slots
                .last()
                .map(|s| s.id() + 1)
                .unwrap_or(new_first_idx);
            let _ = fill_axis(
                &mut new_slots,
                next_id..=last_idx_limit,
                cursor,
                max_cursor,
                &measure,
            );
        }

        Some(new_slots)
    }

    pub(crate) fn rebuild_rows_for_row_scroll(
        &self,
        model: &dyn CanvasModel,
        new_top: i32,
        canvas: CanvasSize,
    ) -> Option<Vec<RowSlot>> {
        Self::rebuild_axis_slots(
            &self.scroll_rows,
            self.frozen_offset_y,
            canvas.h.ceil() as i32,
            new_top,
            LAST_ROW,
            |r| row_height(model, r),
        )
    }

    pub(crate) fn rebuild_cols_for_col_scroll(
        &self,
        model: &dyn CanvasModel,
        new_left: i32,
        canvas: CanvasSize,
    ) -> Option<Vec<ColSlot>> {
        Self::rebuild_axis_slots(
            &self.scroll_cols,
            self.frozen_offset_x,
            canvas.w.ceil() as i32,
            new_left,
            LAST_COLUMN,
            |c| col_width(model, c),
        )
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
