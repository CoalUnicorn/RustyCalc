//! JS-side model bridge.
//!
//! `JsBackedModel` is the wasm impl of `CanvasModel`. It wraps an opaque
//! `IronCalcModelHandle` exposed by JS and routes every trait call through a
//! `wasm_bindgen` extern method. All bridge calls use `(catch, method)` so
//! a JS-side throw becomes `Err(JsValue)` here, never a tab-killing trap.
//!
//! Two failure modes can't be hidden: a method threw, or the returned shape
//! didn't deserialize. Both are counted on `Cell<u64>` and surfaced via
//! `console.warn` exactly once per class per session — enough signal to
//! diagnose a contract drift, not enough to flood the console.

use std::cell::Cell;

use ironcalc_base::types::{CellType, Style};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::types::coord::RCRange;
use crate::CanvasModel;
use crate::CanvasView;

#[wasm_bindgen]
extern "C" {
    pub type IronCalcModelHandle;

    #[wasm_bindgen(catch, method, js_name = "getSelectedSheet")]
    fn get_selected_sheet(this: &IronCalcModelHandle) -> Result<u32, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getSelectedView")]
    fn get_selected_view(this: &IronCalcModelHandle) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getFrozenRowsCount")]
    fn get_frozen_rows_count(this: &IronCalcModelHandle, sheet: u32) -> Result<i32, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getFrozenColumnsCount")]
    fn get_frozen_columns_count(this: &IronCalcModelHandle, sheet: u32) -> Result<i32, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getRowHeight")]
    fn get_row_height(this: &IronCalcModelHandle, sheet: u32, row: i32) -> Result<f64, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getColumnWidth")]
    fn get_column_width(
        this: &IronCalcModelHandle,
        sheet: u32,
        column: i32,
    ) -> Result<f64, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getShowGridLines")]
    fn get_show_grid_lines(this: &IronCalcModelHandle, sheet: u32) -> Result<bool, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getCellStyle")]
    fn get_cell_style(
        this: &IronCalcModelHandle,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getCellType")]
    fn get_cell_type(
        this: &IronCalcModelHandle,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<i32, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getFormattedCellValue")]
    fn get_formatted_cell_value(
        this: &IronCalcModelHandle,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<String, JsValue>;

    // Dep-free console.warn — one binding, used by both the diagnostic
    // surface here and (later) by `painter::canvas` for measure_text errors.
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(s: &str);
}

pub struct JsBackedModel {
    handle: IronCalcModelHandle,
    js_throw_count: Cell<u64>,
    serde_shape_errs: Cell<u64>,
}

impl JsBackedModel {
    pub fn new(handle: IronCalcModelHandle) -> Self {
        Self {
            handle,
            js_throw_count: Cell::new(0),
            serde_shape_errs: Cell::new(0),
        }
    }

    /// Validated cast from a raw JS handle. Caller (orchestrator's
    /// `set_model_js`) propagates the error to JS so a wrong handle
    /// surfaces at the boundary, not on the next render-time method call.
    pub fn try_from_js_value(value: JsValue) -> Result<Self, JsValue> {
        value.dyn_into::<IronCalcModelHandle>().map(Self::new)
    }

    /// `(js_throw_count, serde_shape_errs)`. Diagnostic surface for tests
    /// and any future JS-facing getter on `IronCanvas`.
    pub fn diagnostic_counts(&self) -> (u64, u64) {
        (self.js_throw_count.get(), self.serde_shape_errs.get())
    }

    fn note_js_throw(&self, ctx: &str) {
        let prev = self.js_throw_count.get();
        self.js_throw_count.set(prev + 1);
        if prev == 0 {
            console_warn(&format!(
                "iron-canvas: JS handle method threw ({ctx}); subsequent throws silenced"
            ));
        }
    }

    fn note_serde_err(&self, ctx: &str, err: &serde_wasm_bindgen::Error) {
        let prev = self.serde_shape_errs.get();
        self.serde_shape_errs.set(prev + 1);
        if prev == 0 {
            console_warn(&format!(
                "iron-canvas: JS handle returned non-conforming shape ({ctx}: {err}); \
                 subsequent shape errors silenced"
            ));
        }
    }
}

impl CanvasModel for JsBackedModel {
    fn get_selected_sheet(&self) -> u32 {
        self.handle.get_selected_sheet().unwrap_or_else(|_| {
            self.note_js_throw("getSelectedSheet");
            0
        })
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        let jsv = match self.handle.get_selected_view() {
            Ok(v) => v,
            Err(_) => {
                self.note_js_throw("getSelectedView");
                return None;
            }
        };
        match serde_wasm_bindgen::from_value::<JsSelectedView>(jsv) {
            Ok(j) => Some(j.into_canvas_view()),
            Err(e) => {
                self.note_serde_err("getSelectedView", &e);
                None
            }
        }
    }

    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.handle.get_frozen_rows_count(sheet).ok()
    }

    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.handle.get_frozen_columns_count(sheet).ok()
    }

    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.handle.get_row_height(sheet, row).ok()
    }

    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.handle.get_column_width(sheet, column).ok()
    }

    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.handle.get_show_grid_lines(sheet).ok()
    }

    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        let jsv = self.handle.get_cell_style(sheet, row, column).ok()?;
        match serde_wasm_bindgen::from_value::<Style>(jsv) {
            Ok(s) => Some(s),
            Err(e) => {
                self.note_serde_err("getCellStyle", &e);
                None
            }
        }
    }

    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        self.handle
            .get_cell_type(sheet, row, column)
            .ok()
            .and_then(cell_type_from_discriminant)
    }

    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        self.handle.get_formatted_cell_value(sheet, row, column).ok()
    }
}

#[derive(Deserialize)]
struct JsSelectedView {
    sheet: u32,
    row: i32,
    column: i32,
    range: [i32; 4],
    top_row: i32,
    left_column: i32,
}

impl JsSelectedView {
    fn into_canvas_view(self) -> CanvasView {
        CanvasView {
            sheet: self.sheet,
            row: self.row,
            column: self.column,
            selection: RCRange {
                r1: self.range[0],
                c1: self.range[1],
                r2: self.range[2],
                c2: self.range[3],
            },
            top_row: self.top_row,
            left_column: self.left_column,
        }
    }
}

// Discriminants pinned to `ironcalc_base::types::CellType`. `as i32` keeps
// the mapping bound to the upstream enum — a renumbering breaks the build
// instead of silently mismapping.
const CELL_TYPE_NUMBER: i32 = CellType::Number as i32;
const CELL_TYPE_TEXT: i32 = CellType::Text as i32;
const CELL_TYPE_LOGICAL: i32 = CellType::LogicalValue as i32;
const CELL_TYPE_ERROR: i32 = CellType::ErrorValue as i32;
const CELL_TYPE_ARRAY: i32 = CellType::Array as i32;
const CELL_TYPE_COMPOUND: i32 = CellType::CompoundData as i32;

fn cell_type_from_discriminant(v: i32) -> Option<CellType> {
    match v {
        CELL_TYPE_NUMBER => Some(CellType::Number),
        CELL_TYPE_TEXT => Some(CellType::Text),
        CELL_TYPE_LOGICAL => Some(CellType::LogicalValue),
        CELL_TYPE_ERROR => Some(CellType::ErrorValue),
        CELL_TYPE_ARRAY => Some(CellType::Array),
        CELL_TYPE_COMPOUND => Some(CellType::CompoundData),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_type_from_discriminant_maps_known() {
        assert_eq!(cell_type_from_discriminant(1), Some(CellType::Number));
        assert_eq!(cell_type_from_discriminant(2), Some(CellType::Text));
        assert_eq!(cell_type_from_discriminant(4), Some(CellType::LogicalValue));
        assert_eq!(cell_type_from_discriminant(16), Some(CellType::ErrorValue));
        assert_eq!(cell_type_from_discriminant(64), Some(CellType::Array));
        assert_eq!(cell_type_from_discriminant(128), Some(CellType::CompoundData));
    }

    #[test]
    fn cell_type_from_discriminant_rejects_unknown() {
        assert_eq!(cell_type_from_discriminant(0), None);
        assert_eq!(cell_type_from_discriminant(3), None);
        assert_eq!(cell_type_from_discriminant(-1), None);
        assert_eq!(cell_type_from_discriminant(256), None);
    }

    #[test]
    fn js_selected_view_maps_into_canvas_view() {
        let jsv = JsSelectedView {
            sheet: 2,
            row: 7,
            column: 3,
            range: [5, 1, 12, 4],
            top_row: 6,
            left_column: 2,
        };
        let cv = jsv.into_canvas_view();
        assert_eq!(cv.sheet, 2);
        assert_eq!(cv.row, 7);
        assert_eq!(cv.column, 3);
        assert_eq!(cv.selection.r1, 5);
        assert_eq!(cv.selection.c1, 1);
        assert_eq!(cv.selection.r2, 12);
        assert_eq!(cv.selection.c2, 4);
        assert_eq!(cv.top_row, 6);
        assert_eq!(cv.left_column, 2);
    }
}
