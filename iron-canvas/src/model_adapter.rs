// CanvasModel - read-only worksheet surface the renderer consumes

use ironcalc_base::types::{CellType, Style};
use ironcalc_base::UserModel;

use crate::types::coord::RCRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub selection: RCRange,
    pub top_row: i32,
    pub left_column: i32,
}

pub trait CanvasModel {
    fn get_selected_sheet(&self) -> u32;
    /// `None` is reserved for the JS bridge: it signals the bridge call threw
    /// or the returned shape didn't deserialize. Treat as a transient absence,
    /// not a steady state — the next animation frame will re-query.
    fn get_selected_view(&self) -> Option<CanvasView>;
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String>;

    /// Bulk-fetch cell styles for the contiguous rectangle `range` on `sheet`.
    /// Output is dense, row-major: `out[(row - r1) * cols + (col - c1)]` for
    /// every `(row, col)` in `[r1..=r2] × [c1..=c2]`. `None` entries mean the
    /// per-cell fetch failed (same semantics as `get_cell_style` returning
    /// `None`).
    ///
    /// Default impl loops `get_cell_style` so `UserModel` is byte-for-byte
    /// unchanged. The wasm bridge will override with one JS call per range.
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<Style>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch formatted cell values for the contiguous rectangle `range`
    /// on `sheet`. Same row-major dense layout and `None` semantics as
    /// `get_cell_styles_in`. Default impl loops `get_formatted_cell_value`;
    /// wasm bridge overrides with one JS call per range.
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Option<String>>,
    ) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_formatted_cell_value(sheet, r, c));
            }
        }
    }
}

impl<'a> CanvasModel for UserModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(self)
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        let v = UserModel::get_selected_view(self);
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
        UserModel::get_frozen_rows_count(self, sheet).ok()
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        UserModel::get_frozen_columns_count(self, sheet).ok()
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        UserModel::get_row_height(self, sheet, row).ok()
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        UserModel::get_column_width(self, sheet, column).ok()
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        UserModel::get_show_grid_lines(self, sheet).ok()
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        UserModel::get_cell_style(self, sheet, row, column).ok()
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        UserModel::get_cell_type(self, sheet, row, column).ok()
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        UserModel::get_formatted_cell_value(self, sheet, row, column).ok()
    }
}
