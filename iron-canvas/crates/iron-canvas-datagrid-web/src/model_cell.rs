use std::cell::RefCell;

use iron_canvas_core::{CanvasModel, CanvasView, CellKind, CellStyle, Fetched};
use iron_canvas_datagrid::DataGrid;

/// Interior-mutable `DataGrid` so the wasm handle can mutate the model
/// in place while the orchestrator holds an `Rc` to the same object.
pub struct DataGridModel(RefCell<DataGrid>);

impl DataGridModel {
    pub fn empty() -> Self {
        Self(RefCell::new(DataGrid::builder().build()))
    }

    pub fn replace(&self, grid: DataGrid) {
        self.0.replace(grid);
    }

    /// Run a mutation against the inner grid (used by Stage D mutators).
    pub fn borrow_mut_with<R>(&self, f: impl FnOnce(&mut DataGrid) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    /// Read-only sort snapshot: `(0-based column, ascending)` or `None`.
    /// A dedicated reader because `borrow_mut_with` can't return a borrow.
    pub fn borrow_current_sort(&self) -> Option<(usize, bool)> {
        self.0.borrow().current_sort()
    }
}

// Forward the non-defaulted `CanvasModel` methods. The defaulted bulk
// readers (`get_*_in`) and the `get_extended_cell_style` / header-toggle
// defaults call these per-cell forwarders, so they stay correct without
// explicit forwarding.
impl CanvasModel for DataGridModel {
    fn get_selected_sheet(&self) -> u32 {
        self.0.borrow().get_selected_sheet()
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        self.0.borrow().get_selected_view()
    }
    fn get_frozen_rows_count(&self, s: u32) -> Option<i32> {
        self.0.borrow().get_frozen_rows_count(s)
    }
    fn get_frozen_columns_count(&self, s: u32) -> Option<i32> {
        self.0.borrow().get_frozen_columns_count(s)
    }
    fn get_row_height(&self, s: u32, row: i32) -> Option<f64> {
        self.0.borrow().get_row_height(s, row)
    }
    fn get_column_width(&self, s: u32, col: i32) -> Option<f64> {
        self.0.borrow().get_column_width(s, col)
    }
    fn get_show_grid_lines(&self, s: u32) -> Option<bool> {
        self.0.borrow().get_show_grid_lines(s)
    }
    fn get_cell_style(&self, s: u32, row: i32, col: i32) -> Fetched<CellStyle> {
        self.0.borrow().get_cell_style(s, row, col)
    }
    fn get_cell_type(&self, s: u32, row: i32, col: i32) -> Fetched<CellKind> {
        self.0.borrow().get_cell_type(s, row, col)
    }
    fn get_formatted_cell_value(&self, s: u32, row: i32, col: i32) -> Fetched<String> {
        self.0.borrow().get_formatted_cell_value(s, row, col)
    }
    fn get_column_header_text(&self, s: u32, col: i32) -> Option<String> {
        self.0.borrow().get_column_header_text(s, col)
    }
    fn get_row_header_text(&self, s: u32, row: i32) -> Option<String> {
        self.0.borrow().get_row_header_text(s, row)
    }
}
