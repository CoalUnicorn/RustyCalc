use leptos::prelude::*;

use crate::state::{ModelStore, Split};
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{CanvasModel, CanvasView, CellDecoration, CellKind, CellStyle};
use iron_canvas_ironcalc::convert::{
    cell_decoration_from_extended, cell_type_to_kind, style_to_core,
};

/// Bridges `ModelStore` (a Leptos `StoredValue` holding `UserModel<'static>`)
/// to `iron_canvas::CanvasModel`. Each trait method `with_value`-borrows the
/// current `UserModel` and dispatches through its inherent IronCalc API,
/// converting styling types via the `iron-canvas-ironcalc` bridge (the
/// `impl CanvasModel for UserModel` blanket was removed in EXT-5). The handle
/// (`ModelStore`) is `Copy`, so the adapter is freely `'static` and the
/// wrapping `Rc<dyn CanvasModel>` is stable across the component's lifetime —
/// workbook switches that replace the inner `UserModel` are picked up
/// automatically on the next render-time read.
pub(super) struct WorksheetModelAdapter {
    pub store: ModelStore,
    pub show_headers: Split<bool>,
}

impl CanvasModel for WorksheetModelAdapter {
    fn get_selected_sheet(&self) -> u32 {
        self.store.with_value(|m| m.get_selected_sheet())
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        self.store.with_value(|m| {
            let v = m.get_selected_view();
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
        })
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| m.get_frozen_rows_count(sheet).ok())
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| m.get_frozen_columns_count(sheet).ok())
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.store.with_value(|m| m.get_row_height(sheet, row).ok())
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.store
            .with_value(|m| m.get_column_width(sheet, column).ok())
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.store.with_value(|m| m.get_show_grid_lines(sheet).ok())
    }
    fn get_show_row_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    fn get_show_col_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellStyle> {
        // Merged (dxf-applied) style via the bridge, mirroring IronCalcModel.
        self.store.with_value(|m| {
            m.get_extended_cell_style(sheet, row, column)
                .ok()
                .map(|ext| style_to_core(ext.style))
        })
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellKind> {
        self.store.with_value(|m| {
            m.get_cell_type(sheet, row, column)
                .ok()
                .map(cell_type_to_kind)
        })
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        self.store
            .with_value(|m| m.get_formatted_cell_value(sheet, row, column).ok())
    }
    fn get_extended_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellDecoration> {
        self.store.with_value(|m| {
            let ext = m.get_extended_cell_style(sheet, row, column).ok()?;
            cell_decoration_from_extended(&ext)
        })
    }
}
