//! `iron-canvas-datagrid` — proof that `iron-canvas-core` is engine-agnostic.
//!
//! A trivial in-memory `Vec<Vec<String>>` model renders through the real
//! iron-canvas pipeline with ZERO IronCalc. The only runtime dependency is
//! `iron-canvas-core`; the recorder is a dev-dependency used by the smoke test.

use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{CanvasModel, CanvasView, CellKind, CellStyle};

pub struct DataGrid {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_h: f64,
    pub col_w: f64,
}

impl DataGrid {
    pub fn new(headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self {
            headers,
            rows,
            row_h: 22.0,
            col_w: 96.0,
        }
    }
}

impl CanvasModel for DataGrid {
    fn get_selected_sheet(&self) -> u32 {
        0
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            },
            top_row: 1,
            left_column: 1,
        })
    }

    // Freeze the first data row to demonstrate freeze works without IronCalc.
    fn get_frozen_rows_count(&self, _sheet: u32) -> Option<i32> {
        Some(1)
    }

    fn get_frozen_columns_count(&self, _sheet: u32) -> Option<i32> {
        Some(0)
    }

    fn get_row_height(&self, _sheet: u32, _row: i32) -> Option<f64> {
        Some(self.row_h)
    }

    fn get_column_width(&self, _sheet: u32, _column: i32) -> Option<f64> {
        Some(self.col_w)
    }

    fn get_show_grid_lines(&self, _sheet: u32) -> Option<bool> {
        Some(true)
    }

    fn get_cell_style(&self, _sheet: u32, _row: i32, _column: i32) -> Option<CellStyle> {
        Some(CellStyle::default())
    }

    fn get_cell_type(&self, _sheet: u32, _row: i32, _column: i32) -> Option<CellKind> {
        Some(CellKind::Text)
    }

    fn get_formatted_cell_value(&self, _sheet: u32, row: i32, column: i32) -> Option<String> {
        if row < 1 || column < 1 {
            return None;
        }
        self.rows
            .get((row - 1) as usize)?
            .get((column - 1) as usize)
            .cloned()
    }

    // Data-driven headers: proves model-supplied column labels reach the renderer.
    fn get_column_header_text(&self, _sheet: u32, col: i32) -> Option<String> {
        if col < 1 {
            return None;
        }
        self.headers.get((col - 1) as usize).cloned()
    }
}
