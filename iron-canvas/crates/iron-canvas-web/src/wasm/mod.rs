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

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use ironcalc_base::types as ic;

use crate::wasm::diag::console_warn;
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{
    Alignment, Border, BorderItem, BorderStyle, CellKind, CellStyle, FontStyle,
    HAlign, VAlign,
};
use iron_canvas_core::{CanvasModel, CanvasView};

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

    /// Adopt a raw JS handle as an opaque `IronCalcModelHandle`. Validates
    /// structurally (one duck-tested method) rather than by `instanceof`,
    /// because the handle is module-agnostic — a host may bundle the
    /// IronCalc wasm under any path. Returns a `JsError` (not a bare
    /// `JsValue`) so the JS-side catch sees a real `Error` with a useful
    /// `.message` instead of an opaque `[object Object]`.
    pub fn try_from_js_value(value: JsValue) -> Result<Self, JsError> {
        let probe = JsValue::from_str("getSelectedView");
        let has = js_sys::Reflect::has(&value, &probe).map_err(|_| {
            JsError::new("setModel: argument is not an object (expected an IronCalc Model)")
        })?;
        if !has {
            return Err(JsError::new(
                "setModel: handle missing required method 'getSelectedView' \
                 — expected an IronCalc Model",
            ));
        }
        Ok(Self::new(value.unchecked_into()))
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

    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<CellStyle> {
        let jsv = self.handle.get_cell_style(sheet, row, column).ok()?;
        match serde_wasm_bindgen::from_value::<ic::Style>(jsv) {
            Ok(s) => Some(ic_style_to_core(s)),
            Err(e) => {
                self.note_serde_err("getCellStyle", &e);
                None
            }
        }
    }

    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellKind> {
        self.handle
            .get_cell_type(sheet, row, column)
            .ok()
            .and_then(cell_kind_from_discriminant)
    }

    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        self.handle
            .get_formatted_cell_value(sheet, row, column)
            .ok()
    }

    // TODO(W5): collapse bulk accessors to single JS round-trips once the JS
    // Model handle exposes bulk decoration/style/value APIs. Until then the
    // trait defaults (per-cell loops) run — correct, just chatty.
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
const CELL_TYPE_NUMBER: i32 = ic::CellType::Number as i32;
const CELL_TYPE_TEXT: i32 = ic::CellType::Text as i32;
const CELL_TYPE_LOGICAL: i32 = ic::CellType::LogicalValue as i32;
const CELL_TYPE_ERROR: i32 = ic::CellType::ErrorValue as i32;
const CELL_TYPE_ARRAY: i32 = ic::CellType::Array as i32;
const CELL_TYPE_COMPOUND: i32 = ic::CellType::CompoundData as i32;

fn cell_kind_from_discriminant(v: i32) -> Option<CellKind> {
    match v {
        CELL_TYPE_NUMBER => Some(CellKind::Number),
        CELL_TYPE_TEXT | CELL_TYPE_ARRAY | CELL_TYPE_COMPOUND => Some(CellKind::Text),
        CELL_TYPE_LOGICAL => Some(CellKind::Logical),
        CELL_TYPE_ERROR => Some(CellKind::Error),
        _ => None,
    }
}

/// Convert an IronCalc `Style` (deserialized from JS) to the core `CellStyle`.
/// Mirrors `iron-canvas-ironcalc::convert::style_to_core` — kept local to
/// avoid pulling `iron-canvas-ironcalc` into the web crate's dep tree.
fn ic_style_to_core(s: ic::Style) -> CellStyle {
    CellStyle {
        fill_color: s.fill.color,
        font: FontStyle {
            name: s.font.name,
            size: f64::from(s.font.sz),
            color: s.font.color,
            bold: s.font.b,
            italic: s.font.i,
            underline: s.font.u,
            strike: s.font.strike,
        },
        alignment: s.alignment.map(|a| Alignment {
            horizontal: ic_halign_to_core(a.horizontal),
            vertical: ic_valign_to_core(a.vertical),
            wrap_text: a.wrap_text,
        }),
        border: Border {
            left: s.border.left.map(ic_border_item_to_core),
            right: s.border.right.map(ic_border_item_to_core),
            top: s.border.top.map(ic_border_item_to_core),
            bottom: s.border.bottom.map(ic_border_item_to_core),
            diagonal_up: s.border.diagonal_up,
            diagonal_down: s.border.diagonal_down,
        },
    }
}

fn ic_halign_to_core(h: ic::HorizontalAlignment) -> HAlign {
    match h {
        ic::HorizontalAlignment::Center => HAlign::Center,
        ic::HorizontalAlignment::CenterContinuous => HAlign::CenterContinuous,
        ic::HorizontalAlignment::Distributed => HAlign::Distributed,
        ic::HorizontalAlignment::Fill => HAlign::Fill,
        ic::HorizontalAlignment::General => HAlign::General,
        ic::HorizontalAlignment::Justify => HAlign::Justify,
        ic::HorizontalAlignment::Left => HAlign::Left,
        ic::HorizontalAlignment::Right => HAlign::Right,
    }
}

fn ic_valign_to_core(v: ic::VerticalAlignment) -> VAlign {
    match v {
        ic::VerticalAlignment::Bottom => VAlign::Bottom,
        ic::VerticalAlignment::Center => VAlign::Center,
        ic::VerticalAlignment::Distributed => VAlign::Distributed,
        ic::VerticalAlignment::Justify => VAlign::Justify,
        ic::VerticalAlignment::Top => VAlign::Top,
    }
}

fn ic_border_item_to_core(b: ic::BorderItem) -> BorderItem {
    BorderItem {
        style: ic_border_style_to_core(b.style),
        color: b.color,
    }
}

fn ic_border_style_to_core(s: ic::BorderStyle) -> BorderStyle {
    match s {
        ic::BorderStyle::Thin => BorderStyle::Thin,
        ic::BorderStyle::Medium => BorderStyle::Medium,
        ic::BorderStyle::Thick => BorderStyle::Thick,
        ic::BorderStyle::Double => BorderStyle::Double,
        ic::BorderStyle::Dotted => BorderStyle::Dotted,
        ic::BorderStyle::SlantDashDot => BorderStyle::SlantDashDot,
        ic::BorderStyle::MediumDashed => BorderStyle::MediumDashed,
        ic::BorderStyle::MediumDashDotDot => BorderStyle::MediumDashDotDot,
        ic::BorderStyle::MediumDashDot => BorderStyle::MediumDashDot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_kind_from_discriminant_maps_known() {
        assert_eq!(cell_kind_from_discriminant(1), Some(CellKind::Number));
        assert_eq!(cell_kind_from_discriminant(2), Some(CellKind::Text));
        assert_eq!(cell_kind_from_discriminant(4), Some(CellKind::Logical));
        assert_eq!(cell_kind_from_discriminant(16), Some(CellKind::Error));
        // Array and CompoundData collapse to Text
        assert_eq!(cell_kind_from_discriminant(64), Some(CellKind::Text));
        assert_eq!(cell_kind_from_discriminant(128), Some(CellKind::Text));
    }

    #[test]
    fn cell_kind_from_discriminant_rejects_unknown() {
        assert_eq!(cell_kind_from_discriminant(0), None);
        assert_eq!(cell_kind_from_discriminant(3), None);
        assert_eq!(cell_kind_from_discriminant(-1), None);
        assert_eq!(cell_kind_from_discriminant(256), None);
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

pub mod diag;
