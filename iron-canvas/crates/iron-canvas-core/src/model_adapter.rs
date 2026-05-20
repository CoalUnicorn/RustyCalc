// CanvasModel - read-only worksheet surface the renderer consumes

use std::rc::Rc;

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
    /// `None` signals a transient JS-bridge failure: the bridge call threw
    /// or the returned shape didn't deserialize. The next animation frame
    /// will re-query.
    fn get_selected_view(&self) -> Option<CanvasView>;
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64>;
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool>;
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String>;

    /// Bulk-fetch cell styles for `range` on `sheet`. Output is dense,
    /// row-major: `out[(row - r1) * cols + (col - c1)]`. `None` entries
    /// carry the same fetch-failed meaning as `get_cell_style`.
    ///
    /// Default impl loops the per-cell accessor so `UserModel` keeps its
    /// existing behaviour; the wasm bridge overrides this with a single
    /// JS round-trip per range.
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<Style>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c));
            }
        }
    }

    /// Bulk-fetch formatted cell values for `range` on `sheet`. Same dense
    /// row-major layout and `None`-as-failure semantics as
    /// `get_cell_styles_in`; same default-impl / wasm-override pattern.
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

    /// Bulk-fetch cell types for `range` on `sheet`. Same layout and
    /// semantics as the other `*_in` accessors. Feeds the text pass's
    /// alignment/colour resolution in `CellTextStyle::resolve`.
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellType>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_type(sheet, r, c));
            }
        }
    }
}

/// Forwarding impl so `Orchestrator<S, Rc<JsBackedModel>>` (in the web
/// crate) satisfies the `M: CanvasModel` bound. `?Sized` lets `Rc<dyn
/// CanvasModel>` also satisfy it for callers that prefer dyn dispatch.
impl<T: CanvasModel + ?Sized> CanvasModel for Rc<T> {
    fn get_selected_sheet(&self) -> u32 {
        (**self).get_selected_sheet()
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        (**self).get_selected_view()
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        (**self).get_frozen_rows_count(sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        (**self).get_frozen_columns_count(sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        (**self).get_row_height(sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        (**self).get_column_width(sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        (**self).get_show_grid_lines(sheet)
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        (**self).get_cell_style(sheet, row, column)
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        (**self).get_cell_type(sheet, row, column)
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        (**self).get_formatted_cell_value(sheet, row, column)
    }
    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<Style>>) {
        (**self).get_cell_styles_in(sheet, range, out)
    }
    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Option<String>>,
    ) {
        (**self).get_formatted_cell_values_in(sheet, range, out)
    }
    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Option<CellType>>) {
        (**self).get_cell_types_in(sheet, range, out)
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
