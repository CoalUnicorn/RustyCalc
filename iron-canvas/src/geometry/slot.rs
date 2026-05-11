//! Per-axis visible slots on the painted frame.
//!
//! A `RowSlot` / `ColSlot` is the axis-level peer of `CellSlot`: the index
//! plus the absolute canvas coordinate of its leading edge plus its extent.
//! `PaneSet` stores four vecs of these (frozen / scrollable × row /
//! column) and every pixel↔cell query reads them — no prefix-sum decoding.

use crate::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use crate::CanvasModel;

#[derive(Clone, Copy, Debug)]
pub struct RowSlot {
    pub row: i32,
    /// Absolute canvas Y of this row's top edge.
    pub top: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColSlot {
    pub col: i32,
    /// Absolute canvas X of this column's left edge.
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
