//! Pure-axis machinery: per-axis slot walks, frozen/scroll partitioning,
//! and the row-header width measurement that anchors the cell-area origin.
//!
//! `Chrome` composes these axis-symmetric methods whenever a query spans
//! both axes; everything here knows about only one axis at a time. The
//! blit probe/rebuild paths and the cross-frame slot recycler live in
//! sibling files (`blit_rebuild.rs`, `recycled_slots.rs`) — this module
//! holds only the pure-axis surface.

use crate::CanvasModel;
use crate::geometry::constants::HEADER_COL_WIDTH;
use crate::geometry::slot::{AxisSlots, ColSlot, RowSlot, col_width, row_height};

use super::recycled_slots::RecycledSlots;

/// Approx pixel width per digit at the bold 12px Inter header font.
/// Pessimistic enough that no row label clips inside the strip.
const APPROX_DIGIT_WIDTH_PX: i32 = 8;
/// Padding either side of the row-label inside the header strip.
const HEADER_LABEL_PAD_PX: i32 = 4;

#[derive(Debug)]
pub struct PaneSet {
    pub rows: AxisSlots<RowSlot>,
    pub cols: AxisSlots<ColSlot>,
    /// Resolved row-header labels, in the same order walk_header_strip visits:
    /// rows.frozen ++ rows.scroll. Built in Chrome::build with the model in scope.
    pub row_header_labels: Vec<String>,
    /// Resolved column-header labels, parallel to cols.frozen ++ cols.scroll.
    pub col_header_labels: Vec<String>,
}

impl PaneSet {
    /// Fresh `PaneSet` reusing the previous frame's drained slot Vecs.
    /// Each axis's `frozen_offset` is filled in by `fill_rows` / `fill_cols`.
    pub fn with_recycled(recycled: RecycledSlots) -> Self {
        PaneSet {
            rows: AxisSlots {
                frozen: recycled.frozen_rows,
                scroll: recycled.scroll_rows,
                frozen_offset: 0,
                last_id: 0,
            },
            cols: AxisSlots {
                frozen: recycled.frozen_cols,
                scroll: recycled.scroll_cols,
                frozen_offset: 0,
                last_id: 0,
            },
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

    /// Column mirror of [`resolve_row_labels`], falling back to the A/B/...
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

    /// Populate `rows` (frozen + scroll bands and `frozen_offset`) via
    /// `AxisSlots::fill` (Phase B of `Chrome::build`; see the
    /// [`chrome`](crate::chrome) module docs).
    /// Runs before the row-label measurement, so it does not depend on
    /// `row_header_thickness`.
    pub fn fill_rows(
        &mut self,
        model: &dyn CanvasModel,
        frozen_count: i32,
        origin_y: i32,
        view_top_row: i32,
        last_row: i32,
        canvas_h: f64,
    ) {
        self.rows.fill(
            model,
            frozen_count,
            origin_y,
            view_top_row,
            last_row,
            canvas_h.ceil() as i32,
            row_height,
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
        last_column: i32,
        canvas_w: f64,
    ) {
        self.cols.fill(
            model,
            frozen_count,
            origin_x,
            view_left_column,
            last_column,
            canvas_w.ceil() as i32,
            col_width,
        );
    }

    #[inline]
    pub fn top_row(&self) -> i32 {
        self.rows.top()
    }

    #[inline]
    pub fn left_column(&self) -> i32 {
        self.cols.top()
    }

    #[inline]
    pub fn last_visible_row(&self) -> i32 {
        self.rows.last_visible()
    }

    #[inline]
    pub fn last_visible_col(&self) -> i32 {
        self.cols.last_visible()
    }

    #[inline]
    pub fn row_in_frame(&self, row: i32) -> bool {
        self.rows.contains(row)
    }

    #[inline]
    pub fn col_in_frame(&self, col: i32) -> bool {
        self.cols.contains(col)
    }

    #[inline]
    pub fn row_extent_at(&self, row: i32) -> i32 {
        self.rows.extent_at(row)
    }

    #[inline]
    pub fn col_extent_at(&self, col: i32) -> i32 {
        self.cols.extent_at(col)
    }

    pub fn row_to_y(&self, row: i32) -> i32 {
        self.rows.to_pixel(row)
    }

    pub fn col_to_x(&self, col: i32) -> i32 {
        self.cols.to_pixel(col)
    }
}

/// Decimal digit count, clamped to `>= 1` so a zero input still reserves a slot.
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
