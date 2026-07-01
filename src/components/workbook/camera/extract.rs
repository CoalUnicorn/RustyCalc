//! Source-range extraction: UserModel cells -> a headerless styled DataGrid.
//! Mirrors WorksheetModelAdapter's per-cell calls (same merged-style path),
//! but eagerly over the whole range — the camera is a snapshot, not a live
//! adapter. Colors are resolved here (Color::Theme -> CSS), so theme events
//! must trigger re-extraction.

use ironcalc_base::UserModel;

use iron_canvas_core::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use iron_canvas_datagrid::{Cell, Column, DataGrid};
use iron_canvas_ironcalc::{color_resolver, convert::style_to_core};

use crate::coord::SheetRange;

pub fn extract_grid(m: &UserModel, source: SheetRange) -> DataGrid {
    let sheet = source.sheet;
    let area = source.area.normalized();
    let resolve = color_resolver(m);

    // DataGrid has a single grid-wide row height; the source's first row is
    // the best available default.
    let mut builder = DataGrid::builder()
        .show_headers(false)
        .show_selection(false)
        .default_row_height(
        m.get_row_height(sheet, area.r1)
            .ok()
            .unwrap_or(DEFAULT_ROW_HEIGHT),
    );

    for col in area.columns() {
        builder = builder.column(
            Column::new("").width(
                m.get_column_width(sheet, col)
                    .ok()
                    .unwrap_or(DEFAULT_COL_WIDTH),
            ),
        );
    }

    for row in area.rows() {
        let cells: Vec<Cell> = area
            .columns()
            .map(|col| Cell {
                value: m
                    .get_formatted_cell_value(sheet, row, col)
                    .ok()
                    .unwrap_or_default(),
                style: m
                    .get_extended_cell_style(sheet, row, col)
                    .ok()
                    .map(|ext| style_to_core(ext.style, &resolve)),
            })
            .collect();
        builder = builder.styled_row(cells);
    }

    builder.build()
}
