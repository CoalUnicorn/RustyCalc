use crate::{
    geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT},
    CanvasModel,
};

// Row height for `row`, falling back to `DEFAULT_ROW_HEIGHT`.
// #[inline]
// pub fn row_height(m: &dyn CanvasModel, row: i32) -> i32 {
//     m.get_row_height(m.get_selected_sheet(), row)
//         .unwrap_or(DEFAULT_ROW_HEIGHT as f64) as i32
// }

pub fn row_height(model: &dyn CanvasModel, row: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_row_height(sheet, row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
        .round() as i32
}

/// Column width for `col`, falling back to `DEFAULT_COL_WIDTH`.
pub fn col_width(model: &dyn CanvasModel, col: i32) -> i32 {
    let sheet = model.get_selected_sheet();
    model
        .get_column_width(sheet, col)
        .unwrap_or(DEFAULT_COL_WIDTH)
        .round() as i32
}

/// Convert a 1-based column index to its spreadsheet letter name (A, B, ..., XFD).
///
/// Delegates to `ironcalc_base::expressions::utils::number_to_column` - the
/// single authoritative implementation for this conversion in the codebase.
pub fn col_name(col: i32) -> String {
    ironcalc_base::expressions::utils::number_to_column(col).unwrap_or_default()
}
