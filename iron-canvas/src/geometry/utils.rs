use crate::{
    geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT},
    CanvasModel,
};

/// Row height for `row` on `sheet`, falling back to `DEFAULT_ROW_HEIGHT`.
#[inline]
pub fn row_height(m: &dyn CanvasModel, row: i32) -> f64 {
    m.get_row_height(m.get_selected_sheet(), row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
}

/// Column width for `col` on `sheet`, falling back to `DEFAULT_COL_WIDTH`.
#[inline]
pub fn col_width(m: &dyn CanvasModel, col: i32) -> f64 {
    m.get_column_width(m.get_selected_sheet(), col)
        .unwrap_or(DEFAULT_COL_WIDTH)
}

/// Convert a 1-based column index to its spreadsheet letter name (A, B, ..., XFD).
///
/// Delegates to `ironcalc_base::expressions::utils::number_to_column` - the
/// single authoritative implementation for this conversion in the codebase.
pub fn col_name(col: i32) -> String {
    ironcalc_base::expressions::utils::number_to_column(col).unwrap_or_default()
}
