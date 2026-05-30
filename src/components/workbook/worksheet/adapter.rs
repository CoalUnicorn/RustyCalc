use ironcalc_base::types::{CellType, Style};
use leptos::prelude::*;

use crate::state::{ModelStore, Split};
use iron_canvas_core::{CanvasModel, CanvasView};

/// Bridges `ModelStore` (a Leptos `StoredValue` holding `UserModel<'static>`)
/// to `iron_canvas::CanvasModel`. Each trait method `with_value`-borrows the
/// current `UserModel` and dispatches through its existing `CanvasModel`
/// impl. The handle (`ModelStore`) is `Copy`, so the adapter is freely
/// `'static` and the wrapping `Rc<dyn CanvasModel>` is stable across the
/// component's lifetime — workbook switches that replace the inner
/// `UserModel` are picked up automatically on the next render-time read.
pub(super) struct WorksheetModelAdapter {
    pub store: ModelStore,
    pub show_headers: Split<bool>,
}

impl CanvasModel for WorksheetModelAdapter {
    fn get_selected_sheet(&self) -> u32 {
        self.store.with_value(CanvasModel::get_selected_sheet)
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        self.store.with_value(CanvasModel::get_selected_view)
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| CanvasModel::get_frozen_rows_count(m, sheet))
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| CanvasModel::get_frozen_columns_count(m, sheet))
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.store
            .with_value(|m| CanvasModel::get_row_height(m, sheet, row))
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.store
            .with_value(|m| CanvasModel::get_column_width(m, sheet, column))
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.store
            .with_value(|m| CanvasModel::get_show_grid_lines(m, sheet))
    }
    fn get_show_row_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    fn get_show_col_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        self.store
            .with_value(|m| CanvasModel::get_cell_style(m, sheet, row, column))
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        self.store
            .with_value(|m| CanvasModel::get_cell_type(m, sheet, row, column))
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        self.store
            .with_value(|m| CanvasModel::get_formatted_cell_value(m, sheet, row, column))
    }
}
