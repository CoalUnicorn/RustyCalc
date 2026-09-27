use leptos::prelude::*;

use crate::state::{ModelStore, Split};
use iron_canvas_core::address::RCRange;
use iron_canvas_core::{
    CanvasModel, CanvasView, CellContentQuery, CellDecoration, CellKind, CellLink, CellStyle,
    Fetched,
};
use iron_canvas_ironcalc::color_resolver;
use iron_canvas_ironcalc::convert::{
    cell_decoration_from_extended, cell_type_to_kind, link_to_core, merged_range_to_core,
    style_to_core,
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
    fn get_selected_sheet(&self) -> Option<u32> {
        self.store.with_value(|m| Some(m.get_selected_sheet()))
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
    fn get_row_height(&self, sheet: u32, row: i32) -> Fetched<f64> {
        match self.store.with_value(|m| m.get_row_height(sheet, row)) {
            Ok(h) => Fetched::Value(h),
            // Native model error (persistent, not transient) — treat as
            // "no override", matching the content accessors' convention.
            Err(_) => Fetched::Absent,
        }
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Fetched<f64> {
        match self.store.with_value(|m| m.get_column_width(sheet, column)) {
            Ok(w) => Fetched::Value(w),
            Err(_) => Fetched::Absent,
        }
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Fetched<bool> {
        match self.store.with_value(|m| m.get_show_grid_lines(sheet)) {
            Ok(v) => Fetched::Value(v),
            Err(_) => Fetched::Absent,
        }
    }
    fn get_show_row_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    fn get_show_col_headers(&self, _sheet: u32) -> Option<bool> {
        Some(self.show_headers.get_untracked())
    }
    /// The engine's merged link list, converted once at this boundary. A
    /// native model error is persistent, not transient, so it maps to `None`
    /// (hold the attempt) — never to an empty list, which would leave a
    /// visible link unclickable.
    fn get_sheet_links(&self, sheet: u32) -> Option<Vec<CellLink>> {
        self.store.with_value(|m| {
            let resolve = color_resolver(m);
            m.get_links_list(sheet).ok().map(|links| {
                links
                    .into_iter()
                    .map(|view| {
                        link_to_core(view.link, view.row, view.column, view.dynamic, &resolve)
                    })
                    .collect()
            })
        })
    }

    /// The engine's merged list, converted once at this boundary. A native
    /// model error is persistent, not transient, so it maps to `None` (hold the
    /// attempt) — never to an empty list, which would render a merged region as
    /// separate cells and let a user edit a covered cell.
    fn get_merged_ranges(&self, sheet: u32) -> Option<Vec<RCRange>> {
        self.store.with_value(|m| {
            m.get_merged_cells(sheet)
                .ok()
                .map(|cells| cells.into_iter().filter_map(merged_range_to_core).collect())
        })
    }
}

impl CellContentQuery for WorksheetModelAdapter {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        // Merged (dxf-applied) style via the bridge, mirroring IronCalcModel.
        // The inner `with_value` borrows the native `UserModel`, so a `None`
        // here is a model error, not a JS bridge failure — it maps to `Absent`.
        match self.store.with_value(|m| {
            m.get_extended_cell_style(sheet, row, column)
                .ok()
                .map(|ext| style_to_core(ext.style, &color_resolver(m)))
        }) {
            Some(s) => Fetched::Value(s),
            None => Fetched::Absent,
        }
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind> {
        match self.store.with_value(|m| {
            m.get_cell_type(sheet, row, column)
                .ok()
                .map(cell_type_to_kind)
        }) {
            Some(k) => Fetched::Value(k),
            None => Fetched::Absent,
        }
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String> {
        match self
            .store
            .with_value(|m| m.get_formatted_cell_value(sheet, row, column).ok())
        {
            Some(v) => Fetched::Value(v),
            None => Fetched::Absent,
        }
    }
    fn get_extended_cell_style(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Fetched<CellDecoration> {
        match self.store.with_value(|m| {
            let ext = m.get_extended_cell_style(sheet, row, column).ok()?;
            cell_decoration_from_extended(&ext, &color_resolver(m))
        }) {
            Some(d) => Fetched::Value(d),
            None => Fetched::Absent,
        }
    }
}
