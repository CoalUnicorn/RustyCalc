use std::cell::{Ref, RefCell};

use crate::DataGrid;
use iron_canvas_core::{
    CanvasModel, CanvasView, CellContentQuery, CellKind, CellStyle, Fetched, RCRange,
    forward_methods,
};

/// Interior-mutable `DataGrid` wrapper: the owner can mutate the grid
/// in place while the orchestrator holds an `Rc` to the same object.
pub struct DataGridModel(RefCell<DataGrid>);

impl DataGridModel {
    pub fn empty() -> Self {
        Self(RefCell::new(DataGrid::builder().build()))
    }

    pub fn replace(&self, grid: DataGrid) {
        self.0.replace(grid);
    }

    /// Run a read against the inner grid without exposing the borrow.
    pub fn borrow_with<R>(&self, f: impl FnOnce(&DataGrid) -> R) -> R {
        f(&self.0.borrow())
    }

    /// The inner grid, as the accessor the `forward_methods!` bodies below
    /// call. Returns the guard (not `&DataGrid`) because the owner keeps
    /// mutating the grid while the orchestrator holds the model.
    fn inner(&self) -> Ref<'_, DataGrid> {
        self.0.borrow()
    }

    /// Run a mutation against the inner grid without exposing the borrow.
    pub fn borrow_mut_with<R>(&self, f: impl FnOnce(&mut DataGrid) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

// Forward geometry, view, and header queries to the grid. In particular,
// `last_row` and `last_column` must preserve the grid's finite bounds.
// The macro shares forwarding bodies. This list and the engine's `Rc` lists
// must each be checked when a trait method changes.
impl CanvasModel for DataGridModel {
    forward_methods!(inner, {
        fn get_selected_sheet(&self) -> Option<u32>;
        fn get_selected_view(&self) -> Option<CanvasView>;
        fn get_show_selection(&self) -> bool;
        fn get_frozen_rows_count(&self, s: u32) -> Option<i32>;
        fn get_frozen_columns_count(&self, s: u32) -> Option<i32>;
        fn get_row_height(&self, s: u32, row: i32) -> Fetched<f64>;
        fn get_column_width(&self, s: u32, col: i32) -> Fetched<f64>;
        fn get_show_grid_lines(&self, s: u32) -> Fetched<bool>;
        fn last_row(&self, s: u32) -> i32;
        fn last_column(&self, s: u32) -> i32;
        fn get_column_header_text(&self, s: u32, col: i32) -> Option<String>;
        fn get_row_header_text(&self, s: u32, row: i32) -> Option<String>;
        fn get_show_row_headers(&self, s: u32) -> Option<bool>;
        fn get_show_col_headers(&self, s: u32) -> Option<bool>;
    });
}

impl CellContentQuery for DataGridModel {
    forward_methods!(inner, {
        fn get_cell_style(&self, s: u32, row: i32, col: i32) -> Fetched<CellStyle>;
        fn get_cell_type(&self, s: u32, row: i32, col: i32) -> Fetched<CellKind>;
        fn get_formatted_cell_value(&self, s: u32, row: i32, col: i32) -> Fetched<String>;
    });

    // Forward the bulk readers so the per-frame pane fetch reaches `DataGrid`'s
    // direct-storage `*_in` overrides. Without these, the defaulted trait loops
    // would call the single-cell forwarders above — one `RefCell::borrow()` per
    // cell — instead of one borrow per range. Decorations are intentionally not
    // forwarded: `DataGrid` has no `get_cell_decorations_in` override, so the
    // default loop is already its best path.
    fn get_cell_styles_in(&self, s: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        self.0.borrow().get_cell_styles_in(s, range, out);
    }
    fn get_formatted_cell_values_in(&self, s: u32, range: RCRange, out: &mut Vec<Fetched<String>>) {
        self.0.borrow().get_formatted_cell_values_in(s, range, out);
    }
    fn get_cell_types_in(&self, s: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        self.0.borrow().get_cell_types_in(s, range, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Column, DataGrid};
    use iron_canvas_core::HAlign;

    // A right-aligned column so the style readers exercise `column_default_style`
    // rather than returning bare defaults — otherwise the comparison is trivial.
    fn sample() -> DataGrid {
        DataGrid::builder()
            .column(Column::new("A").align(HAlign::Right))
            .column(Column::new("B"))
            .row(vec!["1".to_string(), "two".to_string()])
            .row(vec!["3".to_string(), "four".to_string()])
            .build()
    }

    // Review 2026-06-13 finding #2: the wrapper's forwarded bulk readers must
    // produce exactly what direct `DataGrid` does — proving the per-frame pane
    // fetch reaches the optimized `*_in` overrides, not the per-cell default
    // loop, now that `DataGridModel` is what the orchestrator actually runs.
    #[test]
    fn wrapper_bulk_readers_match_direct_grid() {
        let grid = sample();
        let model = DataGridModel::empty();
        model.replace(sample());
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };

        let (mut dv, mut wv) = (Vec::new(), Vec::new());
        grid.get_formatted_cell_values_in(0, range, &mut dv);
        model.get_formatted_cell_values_in(0, range, &mut wv);
        assert_eq!(dv, wv, "values diverge between direct grid and wrapper");

        let (mut ds, mut ws) = (Vec::new(), Vec::new());
        grid.get_cell_styles_in(0, range, &mut ds);
        model.get_cell_styles_in(0, range, &mut ws);
        assert_eq!(ds, ws, "styles diverge between direct grid and wrapper");

        let (mut dt, mut wt) = (Vec::new(), Vec::new());
        grid.get_cell_types_in(0, range, &mut dt);
        model.get_cell_types_in(0, range, &mut wt);
        assert_eq!(dt, wt, "types diverge between direct grid and wrapper");
    }
}
