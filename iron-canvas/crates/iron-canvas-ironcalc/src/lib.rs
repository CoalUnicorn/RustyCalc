//! IronCalc engine adapter for `iron-canvas-core`.
//!
//! `IronCalcModel` is a newtype wrapper that implements `CanvasModel` for
//! `ironcalc_base::UserModel`.  Rust's orphan rule prevents implementing a
//! foreign trait (`CanvasModel`) for a foreign type (`UserModel`) outside of
//! the crate that defines the trait, so the direct impl lives in
//! `iron-canvas-core`.  This newtype is the engine-agnostic path — it lets
//! any downstream crate use `IronCalcModel` without importing `ironcalc_base`
//! directly.
//!
//! ## Migration path (EXT-5)
//!
//! 1. Callers switch from `UserModel<'a>` to `IronCalcModel<'a>`.
//! 2. The `impl CanvasModel for UserModel` in `iron-canvas-core` is removed.
//! 3. `iron-canvas-core` drops its `ironcalc_base` dependency.

use iron_canvas_core::{CanvasModel, CanvasView, types::coord::RCRange};
use ironcalc_base::UserModel;
use ironcalc_base::cf_types::ExtendedStyle;
use ironcalc_base::types::{CellType, Style};

/// Newtype wrapper that implements `CanvasModel` for `UserModel`.
///
/// Derefs to `UserModel` so callers can still access IronCalc-specific
/// methods directly.  The `CanvasModel` impl is thin delegation.
pub struct IronCalcModel<'a>(pub UserModel<'a>);

impl<'a> std::ops::Deref for IronCalcModel<'a> {
    type Target = UserModel<'a>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> CanvasModel for IronCalcModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(&self.0)
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let v = UserModel::get_selected_view(&self.0);
        Some(CanvasView {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
            selection: RCRange {
                r1: v.range[0],
                c1: v.range[1],
                r2: v.range[2],
                c2: v.range[3],
            },
            top_row: v.top_row,
            left_column: v.left_column,
        })
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        UserModel::get_frozen_rows_count(&self.0, sheet).ok()
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        UserModel::get_frozen_columns_count(&self.0, sheet).ok()
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        UserModel::get_row_height(&self.0, sheet, row).ok()
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        UserModel::get_column_width(&self.0, sheet, column).ok()
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        UserModel::get_show_grid_lines(&self.0, sheet).ok()
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        UserModel::get_cell_style(&self.0, sheet, row, column).ok()
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        UserModel::get_cell_type(&self.0, sheet, row, column).ok()
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        UserModel::get_formatted_cell_value(&self.0, sheet, row, column).ok()
    }
    fn get_extended_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<ExtendedStyle> {
        UserModel::get_extended_cell_style(&self.0, sheet, row, column).ok()
    }
}
