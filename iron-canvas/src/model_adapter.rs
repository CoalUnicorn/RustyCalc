// CanvasModel - read-only worksheet surface the renderer consumes

use ironcalc_base::types::{CellType, Style};
use ironcalc_base::UserModel;

use crate::types::coord::RCRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub range: RCRange,
    pub top_row: i32,
    pub left_column: i32,
}

pub trait CanvasModel {
    fn get_selected_sheet(&self) -> u32;
    fn get_selected_view(&self) -> CanvasView;
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String>;
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String>;
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32)
        -> Result<String, String>;
}

impl<'a> CanvasModel for UserModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(self)
    }
    fn get_selected_view(&self) -> CanvasView {
        let v = UserModel::get_selected_view(self);
        CanvasView {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
            range: RCRange {
                r1: v.range[0],
                c1: v.range[1],
                r2: v.range[2],
                c2: v.range[3],
            },
            top_row: v.top_row,
            left_column: v.left_column,
        }
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_rows_count(self, sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_columns_count(self, sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String> {
        UserModel::get_row_height(self, sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String> {
        UserModel::get_column_width(self, sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String> {
        UserModel::get_show_grid_lines(self, sheet)
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String> {
        UserModel::get_cell_style(self, sheet, row, column)
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String> {
        UserModel::get_cell_type(self, sheet, row, column)
    }
    fn get_formatted_cell_value(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<String, String> {
        UserModel::get_formatted_cell_value(self, sheet, row, column)
    }
}
