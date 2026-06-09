use crate::DataGrid;
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{CanvasModel, CanvasView, CellContentQuery, CellKind, CellStyle, Fetched};

impl CanvasModel for DataGrid {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let [r1, c1, r2, c2] = self.selection_raw();
        Some(CanvasView {
            sheet: 0,
            row: self.active_row_raw(),
            column: self.active_col_raw(),
            selection: RCRange { r1, c1, r2, c2 },
            top_row: self.top_row_raw(),
            left_column: self.left_col_raw(),
        })
    }
    fn get_frozen_rows_count(&self, _s: u32) -> Option<i32> {
        Some(if self.frozen_header_enabled() { 1 } else { 0 })
    }
    fn get_frozen_columns_count(&self, _s: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _s: u32, _row: i32) -> Option<f64> {
        Some(self.default_row_height())
    }
    fn get_column_width(&self, _s: u32, column: i32) -> Option<f64> {
        if column < 1 {
            return Some(96.0); // row-header gutter — standard column width
        }
        Some(self.column_width_px((column - 1) as usize))
    }
    fn get_show_grid_lines(&self, _s: u32) -> Option<bool> {
        Some(true)
    }
    fn get_column_header_text(&self, _s: u32, col: i32) -> Option<String> {
        if col < 1 {
            return None;
        }
        self.column_header((col - 1) as usize).map(str::to_owned)
    }
    fn get_row_header_text(&self, _s: u32, _row: i32) -> Option<String> {
        // DataGrid has no row-header customization — always use numeric labels.
        // Override is explicit (instead of relying on the trait default) so the
        // symmetry with `get_column_header_text` is visible at the impl site.
        None
    }
}

impl CellContentQuery for DataGrid {
    fn get_cell_style(&self, _s: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        if row < 1 || column < 1 {
            return Fetched::Value(CellStyle::default());
        }
        match self.cell_style((row - 1) as usize, (column - 1) as usize) {
            Some(st) => Fetched::Value(st.clone()),
            None => Fetched::Value(self.column_default_style((column - 1) as usize)),
        }
    }
    fn get_cell_type(&self, _s: u32, _row: i32, _col: i32) -> Fetched<CellKind> {
        Fetched::Value(CellKind::Text)
    }
    fn get_formatted_cell_value(&self, _s: u32, row: i32, column: i32) -> Fetched<String> {
        if row < 1 || column < 1 {
            return Fetched::Absent;
        }
        match self.cell_value((row - 1) as usize, (column - 1) as usize) {
            Some(v) => Fetched::Value(v.to_owned()),
            None => Fetched::Absent,
        }
    }
}
