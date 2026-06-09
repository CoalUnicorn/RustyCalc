// Shared `TestModel` — single configurable `CanvasModel` implementation
// covering everything the integration tests need: scroll position, frozen
// panes, per-row/col size overrides, mutable cell values, and a default
// row range that emits non-empty `"R{n}"` values.
//
// Two construction styles coexist: chainable `with_*` builders for setup,
// and `set_*` methods for mid-test mutation through shared references
// (the renderer reads through `&dyn CanvasModel`, so interior mutability
// is required for any test that rebuilds `Chrome::next` after a state
// change).
//
// `#[allow(dead_code)]` on the module level: every test binary compiles
// `mod common;` independently, so any helper not used by that binary
// would otherwise warn.

#![allow(dead_code)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};

use iron_canvas_core::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use iron_canvas_core::{CanvasModel, CanvasSize, CanvasView, CellContentQuery, RCRange};
use iron_canvas_core::{CellDecoration, CellKind, CellStyle, Fetched};

pub struct TestModel {
    sheet: Cell<u32>,
    frozen_rows: Cell<i32>,
    frozen_cols: Cell<i32>,
    default_row_height: Cell<f64>,
    default_col_width: Cell<f64>,
    row_height_overrides: RefCell<BTreeMap<i32, f64>>,
    col_width_overrides: RefCell<BTreeMap<i32, f64>>,
    cell_values: RefCell<HashMap<(i32, i32), String>>,
    decorations: RefCell<HashMap<(i32, i32), CellDecoration>>,
    column_headers: RefCell<HashMap<i32, String>>,
    /// When > 0, rows `1..=data_until` return `"R{row}"` for any column
    /// not explicitly set via `set_cell`. Lets a test populate a synthetic
    /// data band without enumerating cells.
    data_until: Cell<i32>,
    top_row: Cell<i32>,
    left_column: Cell<i32>,
    active_row: Cell<i32>,
    active_col: Cell<i32>,
    selection: Cell<RCRange>,
    show_grid: Cell<bool>,
    show_row_headers: Cell<bool>,
    show_col_headers: Cell<bool>,
    /// When set, `get_formatted_cell_value` reports a transient
    /// `Fetched::BridgeFailed` — simulating a JS-bridge throw so tests can
    /// exercise the active-cell repaint's atomic-skip path.
    value_bridge_fail: Cell<bool>,
}

impl Default for TestModel {
    fn default() -> Self {
        Self {
            sheet: Cell::new(0),
            frozen_rows: Cell::new(0),
            frozen_cols: Cell::new(0),
            default_row_height: Cell::new(DEFAULT_ROW_HEIGHT),
            default_col_width: Cell::new(DEFAULT_COL_WIDTH),
            row_height_overrides: RefCell::default(),
            col_width_overrides: RefCell::default(),
            cell_values: RefCell::default(),
            decorations: RefCell::default(),
            column_headers: RefCell::default(),
            data_until: Cell::new(0),
            top_row: Cell::new(1),
            left_column: Cell::new(1),
            active_row: Cell::new(1),
            active_col: Cell::new(1),
            selection: Cell::new(RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            }),
            show_grid: Cell::new(true),
            show_row_headers: Cell::new(true),
            show_col_headers: Cell::new(true),
            value_bridge_fail: Cell::new(false),
        }
    }
}

impl TestModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Uniform 20px row × 80px column grid. The dimensions every
    /// recorder-based suite in this crate (chrome_invariants, scroll_blit,
    /// paint_skip, orchestrator_regimes, frame_kind) was calibrated
    /// against — visible-row counts, blit overlap arithmetic, and
    /// hit-test pixel positions all depend on those exact numbers.
    /// `canvas_model.rs` keeps the production defaults (`new()`).
    pub fn synthetic_grid() -> Self {
        Self::new()
            .with_default_row_height(20.0)
            .with_default_col_width(80.0)
    }

    pub fn with_sheet(self, sheet: u32) -> Self {
        self.sheet.set(sheet);
        self
    }
    pub fn with_frozen_rows(self, n: i32) -> Self {
        self.frozen_rows.set(n);
        self
    }
    pub fn with_frozen_cols(self, n: i32) -> Self {
        self.frozen_cols.set(n);
        self
    }
    pub fn with_frozen(self, rows: i32, cols: i32) -> Self {
        self.frozen_rows.set(rows);
        self.frozen_cols.set(cols);
        self
    }
    pub fn with_top_row(self, r: i32) -> Self {
        self.top_row.set(r);
        self
    }
    pub fn with_left_column(self, c: i32) -> Self {
        self.left_column.set(c);
        self
    }
    pub fn with_active(self, row: i32, col: i32) -> Self {
        self.active_row.set(row);
        self.active_col.set(col);
        self.selection.set(RCRange {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        });
        self
    }
    pub fn with_selection(self, range: [i32; 4]) -> Self {
        self.active_row.set(range[0]);
        self.active_col.set(range[1]);
        self.selection.set(RCRange::from(range));
        self
    }
    pub fn with_data_until(self, row: i32) -> Self {
        self.data_until.set(row);
        self
    }
    pub fn with_column_header(self, col: i32, text: &str) -> Self {
        self.column_headers
            .borrow_mut()
            .insert(col, text.to_string());
        self
    }
    pub fn with_default_row_height(self, h: f64) -> Self {
        self.default_row_height.set(h);
        self
    }
    pub fn with_default_col_width(self, w: f64) -> Self {
        self.default_col_width.set(w);
        self
    }
    pub fn with_show_grid(self, b: bool) -> Self {
        self.show_grid.set(b);
        self
    }
    pub fn with_hidden_row_headers(self) -> Self {
        self.show_row_headers.set(false);
        self
    }
    pub fn with_hidden_col_headers(self) -> Self {
        self.show_col_headers.set(false);
        self
    }

    pub fn set_top_row(&self, r: i32) {
        self.top_row.set(r);
    }
    pub fn set_left_column(&self, c: i32) {
        self.left_column.set(c);
    }
    pub fn set_active(&self, row: i32, col: i32) {
        self.active_row.set(row);
        self.active_col.set(col);
        self.selection.set(RCRange {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        });
    }
    pub fn set_selection(&self, range: [i32; 4]) {
        self.active_row.set(range[0]);
        self.active_col.set(range[1]);
        self.selection.set(RCRange::from(range));
    }
    pub fn set_frozen_rows(&self, n: i32) {
        self.frozen_rows.set(n);
    }
    pub fn set_frozen_cols(&self, n: i32) {
        self.frozen_cols.set(n);
    }
    pub fn set_row_height(&self, row: i32, h: f64) {
        self.row_height_overrides.borrow_mut().insert(row, h);
    }
    pub fn set_col_width(&self, col: i32, w: f64) {
        self.col_width_overrides.borrow_mut().insert(col, w);
    }
    pub fn set_cell(&self, row: i32, col: i32, value: &str) {
        self.cell_values
            .borrow_mut()
            .insert((row, col), value.to_string());
    }
    pub fn set_decoration(&self, row: i32, col: i32, deco: CellDecoration) {
        self.decorations.borrow_mut().insert((row, col), deco);
    }
    pub fn set_data_until(&self, row: i32) {
        self.data_until.set(row);
    }
    pub fn set_value_bridge_fail(&self, fail: bool) {
        self.value_bridge_fail.set(fail);
    }
    pub fn set_sheet(&self, sheet: u32) {
        self.sheet.set(sheet);
    }

    pub fn selection_range(&self) -> RCRange {
        self.selection.get()
    }
}

impl CanvasModel for TestModel {
    fn get_selected_sheet(&self) -> u32 {
        self.sheet.get()
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: self.sheet.get(),
            row: self.active_row.get(),
            column: self.active_col.get(),
            selection: self.selection.get(),
            top_row: self.top_row.get(),
            left_column: self.left_column.get(),
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_rows.get())
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_cols.get())
    }
    fn get_row_height(&self, _: u32, row: i32) -> Option<f64> {
        Some(
            self.row_height_overrides
                .borrow()
                .get(&row)
                .copied()
                .unwrap_or_else(|| self.default_row_height.get()),
        )
    }
    fn get_column_width(&self, _: u32, col: i32) -> Option<f64> {
        Some(
            self.col_width_overrides
                .borrow()
                .get(&col)
                .copied()
                .unwrap_or_else(|| self.default_col_width.get()),
        )
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(self.show_grid.get())
    }
    fn get_show_row_headers(&self, _: u32) -> Option<bool> {
        Some(self.show_row_headers.get())
    }
    fn get_show_col_headers(&self, _: u32) -> Option<bool> {
        Some(self.show_col_headers.get())
    }
    fn get_column_header_text(&self, _sheet: u32, column: i32) -> Option<String> {
        self.column_headers.borrow().get(&column).cloned()
    }
}

impl CellContentQuery for TestModel {
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
        Fetched::Value(CellStyle::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
        Fetched::Value(CellKind::Text)
    }
    fn get_extended_cell_style(&self, _: u32, row: i32, col: i32) -> Fetched<CellDecoration> {
        match self.decorations.borrow().get(&(row, col)).cloned() {
            Some(d) => Fetched::Value(d),
            None => Fetched::Absent,
        }
    }
    fn get_formatted_cell_value(&self, _: u32, row: i32, col: i32) -> Fetched<String> {
        if self.value_bridge_fail.get() {
            return Fetched::BridgeFailed;
        }
        if let Some(v) = self.cell_values.borrow().get(&(row, col)) {
            return Fetched::Value(v.clone());
        }
        let data_until = self.data_until.get();
        if data_until > 0 && (1..=data_until).contains(&row) {
            return Fetched::Value(format!("R{row}"));
        }
        Fetched::Value(String::new())
    }
}

pub fn canvas_default() -> CanvasSize {
    CanvasSize { w: 600.0, h: 400.0 }
}

pub fn canvas_large() -> CanvasSize {
    CanvasSize {
        w: 1000.0,
        h: 800.0,
    }
}
