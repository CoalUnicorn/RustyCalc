//! Per-axis visible slots on the painted frame.
//!
//! A slot carries the index, the absolute canvas coordinate of its leading
//! edge, and its extent. `PaneSet` holds four vecs (frozen/scrollable ×
//! row/column); every pixel↔cell query reads them directly, no prefix-sum
//! decoding.

use crate::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use crate::CanvasModel;

#[derive(Clone, Copy, Debug)]
pub struct RowSlot {
    pub row: i32,
    /// Absolute canvas Y, not relative to any pane.
    pub top: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColSlot {
    pub col: i32,
    /// Absolute canvas X, not relative to any pane.
    pub left: i32,
    pub width: i32,
}

impl RowSlot {
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.top + self.height
    }
}

impl ColSlot {
    #[inline]
    pub fn right(&self) -> i32 {
        self.left + self.width
    }
}

pub(crate) fn row_height(model: &dyn CanvasModel, row: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_row_height(sheet, row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
        .round() as i32
}

pub(crate) fn col_width(model: &dyn CanvasModel, col: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_column_width(sheet, col)
        .unwrap_or(DEFAULT_COL_WIDTH)
        .round() as i32
}
