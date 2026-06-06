//! Pure-axis machinery: per-axis slot walks, frozen/scroll partitioning,
//! and the row-header width measurement that anchors the cell-area origin.
//!
//! `Chrome` composes these axis-symmetric methods whenever a query spans
//! both axes; everything here knows about only one axis at a time. The
//! blit probe/rebuild paths and the cross-frame slot recycler live in
//! sibling files (`blit_rebuild.rs`, `recycled_slots.rs`) — this module
//! holds only the pure-axis surface.

use crate::CanvasModel;
use crate::geometry::constants::{FROZEN_SEP, HEADER_COL_WIDTH, LAST_COLUMN, LAST_ROW};
use crate::geometry::slot::{
    AxisSlot, ColSlot, RowSlot, boundary_at, col_width, fill_axis, last_visible_id, pixel_to_id,
    row_height, scroll_first, slot_at, top_id,
};

use super::recycled_slots::RecycledSlots;

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding either side of the row-label inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

#[derive(Debug)]
pub struct PaneSet {
    pub frozen_rows: Vec<RowSlot>,
    pub scroll_rows: Vec<RowSlot>,
    pub frozen_offset_y: i32,
    pub frozen_cols: Vec<ColSlot>,
    pub scroll_cols: Vec<ColSlot>,
    pub frozen_offset_x: i32,
    /// Resolved row-header labels, in the same order walk_header_strip visits:
    /// frozen_rows ++ scroll_rows. Built in Chrome::build with the model in scope.
    pub row_header_labels: Vec<String>,
    /// Resolved column-header labels, parallel to frozen_cols ++ scroll_cols.
    pub col_header_labels: Vec<String>,
}

impl PaneSet {
    /// Fresh `PaneSet` reusing the previous frame's drained slot Vecs.
    /// `frozen_offset_*` are filled in by `fill_rows` / `fill_cols`.
    pub fn with_recycled(recycled: RecycledSlots) -> Self {
        PaneSet {
            frozen_rows: recycled.frozen_rows,
            scroll_rows: recycled.scroll_rows,
            frozen_offset_y: 0,
            frozen_cols: recycled.frozen_cols,
            scroll_cols: recycled.scroll_cols,
            frozen_offset_x: 0,
            row_header_labels: Vec::new(),
            col_header_labels: Vec::new(),
        }
    }

    /// Resolve `frozen ++ scroll` header labels in walk_header_strip order:
    /// a model override, else the 1-based row number. The Fresh build and the
    /// blit rebuild both call this, so the two paths can never drift out of the
    /// slot order their painters zip against.
    pub(crate) fn resolve_row_labels(
        model: &dyn CanvasModel,
        sheet: u32,
        frozen: &[RowSlot],
        scroll: &[RowSlot],
    ) -> Vec<String> {
        frozen
            .iter()
            .chain(scroll.iter())
            .map(|s| {
                model
                    .get_row_header_text(sheet, s.row)
                    .unwrap_or_else(|| s.row.to_string())
            })
            .collect()
    }

    /// Column mirror of [`resolve_row_labels`], falling back to the A/B/C…
    /// spreadsheet name.
    pub(crate) fn resolve_col_labels(
        model: &dyn CanvasModel,
        sheet: u32,
        frozen: &[ColSlot],
        scroll: &[ColSlot],
    ) -> Vec<String> {
        frozen
            .iter()
            .chain(scroll.iter())
            .map(|s| {
                model
                    .get_column_header_text(sheet, s.col)
                    .unwrap_or_else(|| crate::geometry::utils::col_name(s.col))
            })
            .collect()
    }

    /// Populate `frozen_rows`, `scroll_rows`, and `frozen_offset_y`
    /// (Phase B of `Chrome::build`; see `ARCHITECTURE.md`). Runs before
    /// the row-label measurement, so it does not depend on
    /// `row_header_thickness`. Reads `FROZEN_SEP` directly because the
    /// gap between frozen and scroll bands is the row axis's concern.
    pub fn fill_rows(
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
    pub fn fill_cols(
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
    pub fn frozen_rows_count(&self) -> i32 {
        self.frozen_rows.len() as i32
    }

    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
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
    pub fn top_row(&self) -> i32 {
        top_id(&self.scroll_rows)
    }

    #[inline]
    pub fn left_column(&self) -> i32 {
        top_id(&self.scroll_cols)
    }

    #[inline]
    pub fn last_visible_row(&self) -> i32 {
        last_visible_id(&self.scroll_rows)
    }

    #[inline]
    pub fn last_visible_col(&self) -> i32 {
        last_visible_id(&self.scroll_cols)
    }

    #[inline]
    pub fn row_in_frame(&self, row: i32) -> bool {
        self.row_slot(row).is_some()
    }

    #[inline]
    pub fn col_in_frame(&self, col: i32) -> bool {
        self.col_slot(col).is_some()
    }

    #[inline]
    pub fn row_extent_at(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.extent()).unwrap_or(0)
    }

    #[inline]
    pub fn col_extent_at(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.extent()).unwrap_or(0)
    }

    pub fn row_to_y(&self, row: i32) -> i32 {
        self.row_slot(row).map(|s| s.start()).unwrap_or(0)
    }

    pub fn col_to_x(&self, col: i32) -> i32 {
        self.col_slot(col).map(|s| s.start()).unwrap_or(0)
    }

    pub fn pixel_to_row(&self, y: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_rows, &self.scroll_rows, y)
    }

    pub fn pixel_to_col(&self, x: i32) -> Option<i32> {
        pixel_to_id(&self.frozen_cols, &self.scroll_cols, x)
    }

    pub fn row_boundary_at(&self, y: i32, tolerance: i32) -> Option<i32> {
        boundary_at(&self.frozen_rows, &self.scroll_rows, y, tolerance)
    }

    pub fn col_boundary_at(&self, x: i32, tolerance: i32) -> Option<i32> {
        boundary_at(&self.frozen_cols, &self.scroll_cols, x, tolerance)
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
pub fn measure_row_header_width(max_visible_row: i32) -> i32 {
    let digits = digit_count(max_visible_row);
    let approx = digits * APPROX_DIGIT_WIDTH_PX + 2 * HEADER_LABEL_PAD_PX;
    approx.max(HEADER_COL_WIDTH)
}
