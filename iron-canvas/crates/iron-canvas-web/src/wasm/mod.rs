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

use std::cell::{Cell, RefCell};

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use ironcalc_base::types as ic;

use crate::wasm::diag::console_warn;
use iron_canvas_core::types::coord::RCRange;
use iron_canvas_core::{
    Alignment, Border, BorderItem, BorderStyle, CellKind, CellStyle, FontStyle, HAlign, VAlign,
};
use iron_canvas_core::{CanvasModel, CanvasView, CellContentQuery, Fetched};

/// Local mirror of `iron-canvas-ironcalc::convert::color_to_css` — kept here,
/// like the rest of `ic_style_to_core`, to avoid pulling that crate into the
/// web crate's dep tree. Resolves theme slots; `None` for `Color::None`.
fn ic_color_to_css(c: &ic::Color, theme: &ic::Theme) -> Option<String> {
    let rgb = c.to_rgb(theme);
    (!rgb.is_empty()).then_some(rgb)
}

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

    // Batched range accessors (optional on the host). A dense, row-major array
    // of length `(r2-r1+1)*(c2-c1+1)` collapses a pane fetch to one boundary
    // crossing. Presence is probed once in `new`; absence falls back per-cell.
    #[wasm_bindgen(catch, method, js_name = "getCellStylesIn")]
    fn get_cell_styles_in(
        this: &IronCalcModelHandle,
        sheet: u32,
        r1: i32,
        c1: i32,
        r2: i32,
        c2: i32,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getFormattedCellValuesIn")]
    fn get_formatted_cell_values_in(
        this: &IronCalcModelHandle,
        sheet: u32,
        r1: i32,
        c1: i32,
        r2: i32,
        c2: i32,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, method, js_name = "getCellTypesIn")]
    fn get_cell_types_in(
        this: &IronCalcModelHandle,
        sheet: u32,
        r1: i32,
        c1: i32,
        r2: i32,
        c2: i32,
    ) -> Result<JsValue, JsValue>;

    // Workbook theme, needed to resolve `Color::Theme(idx, tint)` to CSS.
    // Optional on the host; absence falls back to the Office default theme.
    #[wasm_bindgen(catch, method, js_name = "getTheme")]
    fn get_theme(this: &IronCalcModelHandle) -> Result<JsValue, JsValue>;
}

pub struct JsBackedModel {
    handle: IronCalcModelHandle,
    js_throw_count: Cell<u64>,
    serde_shape_errs: Cell<u64>,
    // Batched-accessor capability, probed once at construction (D-1). A static
    // structural fact, so it lives in a flag — never re-probed and never
    // conflated with a runtime throw.
    has_styles_in: bool,
    has_values_in: bool,
    has_types_in: bool,
    has_get_theme: bool,
    // Workbook theme, fetched lazily and cached for the model's lifetime
    // (user decision: cache once, explicit refresh). The host must call
    // `IronCanvas.themeChanged()` after `model.setTheme(...)` — a stale
    // cache silently misrenders theme colors, and that is a host bug, not
    // a recoverable bridge failure.
    theme: RefCell<Option<ic::Theme>>,
}

impl JsBackedModel {
    pub fn new(handle: IronCalcModelHandle) -> Self {
        let has_styles_in = Self::has_method(&handle, "getCellStylesIn");
        let has_values_in = Self::has_method(&handle, "getFormattedCellValuesIn");
        let has_types_in = Self::has_method(&handle, "getCellTypesIn");
        let has_get_theme = Self::has_method(&handle, "getTheme");
        Self {
            handle,
            js_throw_count: Cell::new(0),
            serde_shape_errs: Cell::new(0),
            has_styles_in,
            has_values_in,
            has_types_in,
            has_get_theme,
            theme: RefCell::new(None),
        }
    }

    /// Drop the cached workbook theme; the next style conversion refetches.
    /// The host's repaint contract: call this (via `IronCanvas.themeChanged`)
    /// after `model.setTheme(...)`, then mark content dirty — every style
    /// fingerprint changes with the theme.
    pub fn theme_changed(&self) {
        self.theme.replace(None);
    }

    /// Run `f` against the cached workbook theme, filling the cache on first
    /// use. A throw, a bad shape, or a host without `getTheme` all cache the
    /// Office default — strictly better than dropping theme colors, and
    /// recoverable via `theme_changed()` once the host fixes itself.
    /// Holds the `RefCell` borrow across `f`; `f` must not re-enter the theme
    /// cache (style conversion never does).
    fn with_theme<T>(&self, f: impl FnOnce(&ic::Theme) -> T) -> T {
        let mut slot = self.theme.borrow_mut();
        let theme = slot.get_or_insert_with(|| self.fetch_theme());
        f(theme)
    }

    fn fetch_theme(&self) -> ic::Theme {
        if !self.has_get_theme {
            return ic::Theme::default();
        }
        let Some(jsv) = self.note_throw("getTheme", self.handle.get_theme()) else {
            return ic::Theme::default();
        };
        match serde_wasm_bindgen::from_value(jsv) {
            Ok(t) => t,
            Err(e) => {
                self.note_serde_err("getTheme", &e);
                ic::Theme::default()
            }
        }
    }

    /// Duck-test a method's presence on the handle. Structural probe via
    /// `Reflect::has` — a missing method is a static absence, not a throw.
    fn has_method(handle: &IronCalcModelHandle, name: &str) -> bool {
        js_sys::Reflect::has(handle.as_ref(), &JsValue::from_str(name)).unwrap_or(false)
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

    /// Funnel a bridge call's `Result` into the `Option` the `CanvasModel`
    /// trait wants, counting any throw on the way through. Every JS-handle
    /// method routes its result here so a thrown error is recorded by
    /// `note_js_throw` (counter + warn-once) instead of being silently
    /// dropped by a bare `.ok()`.
    fn note_throw<T>(&self, ctx: &str, result: Result<T, JsValue>) -> Option<T> {
        result.inspect_err(|_| self.note_js_throw(ctx)).ok()
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

    /// Per-cell style fill — the trait default's body, reachable as a real
    /// method so a flag-miss or batched-failure can degrade to today's exact
    /// behaviour. (`super` can't reach a trait default.)
    fn styles_in_per_cell(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_style(sheet, r, c));
            }
        }
    }

    fn values_in_per_cell(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<String>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_formatted_cell_value(sheet, r, c));
            }
        }
    }

    fn types_in_per_cell(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        out.clear();
        for r in range.r1..=range.r2 {
            for c in range.c1..=range.c2 {
                out.push(self.get_cell_type(sheet, r, c));
            }
        }
    }
}

impl CanvasModel for JsBackedModel {
    fn get_selected_sheet(&self) -> u32 {
        self.note_throw("getSelectedSheet", self.handle.get_selected_sheet())
            .unwrap_or(0)
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        let jsv = self.note_throw("getSelectedView", self.handle.get_selected_view())?;
        match serde_wasm_bindgen::from_value::<JsSelectedView>(jsv) {
            Ok(j) => Some(j.into_canvas_view()),
            Err(e) => {
                self.note_serde_err("getSelectedView", &e);
                None
            }
        }
    }

    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.note_throw(
            "getFrozenRowsCount",
            self.handle.get_frozen_rows_count(sheet),
        )
    }

    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.note_throw(
            "getFrozenColumnsCount",
            self.handle.get_frozen_columns_count(sheet),
        )
    }

    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.note_throw("getRowHeight", self.handle.get_row_height(sheet, row))
    }

    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.note_throw(
            "getColumnWidth",
            self.handle.get_column_width(sheet, column),
        )
    }

    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.note_throw("getShowGridLines", self.handle.get_show_grid_lines(sheet))
    }
}

impl CellContentQuery for JsBackedModel {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        // Mirror IronCalcModel's dxf-MERGED style: the JS `getCellStyle` extern
        // must return the conditional-format-merged style so the fingerprint
        // hashes what is painted. If JS returns the base style, CF cells paint
        // unmerged here — a known parity gap with the native adapter.
        //
        // A JS throw or a non-conforming payload is a transient bridge failure,
        // not an empty cell — the next frame re-queries. `getCellStyle` never
        // legitimately answers "absent" (a blank cell still has a base style),
        // so there is no `Absent` arm here.
        let Some(jsv) = self.note_throw(
            "getCellStyle",
            self.handle.get_cell_style(sheet, row, column),
        ) else {
            return Fetched::BridgeFailed;
        };
        match serde_wasm_bindgen::from_value::<ic::Style>(jsv) {
            Ok(s) => Fetched::Value(self.with_theme(|t| ic_style_to_core(s, t))),
            Err(e) => {
                self.note_serde_err("getCellStyle", &e);
                Fetched::BridgeFailed
            }
        }
    }

    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind> {
        // Two failures with opposite lifetimes must not share a variant. A
        // *throw* is transient — `BridgeFailed`, so the caller holds prior
        // pixels and re-queries next frame. A successful call carrying a
        // discriminant the core enum doesn't model is *persistent* (re-querying
        // returns the same value); routing it through `BridgeFailed` would
        // suppress the active-cell overlay every frame. It maps to `Absent`,
        // letting the renderer's `unwrap_or(CellKind::Text)` own the fallback —
        // matching the native adapter, where a model error is also `Absent`.
        let Some(disc) =
            self.note_throw("getCellType", self.handle.get_cell_type(sheet, row, column))
        else {
            return Fetched::BridgeFailed;
        };
        match cell_kind_from_discriminant(disc) {
            Some(k) => Fetched::Value(k),
            None => Fetched::Absent,
        }
    }

    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String> {
        match self.note_throw(
            "getFormattedCellValue",
            self.handle.get_formatted_cell_value(sheet, row, column),
        ) {
            Some(v) => Fetched::Value(v),
            None => Fetched::BridgeFailed,
        }
    }

    fn get_cell_styles_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellStyle>>) {
        // D-1: structural absence routes to the per-cell path, untouched.
        if !self.has_styles_in {
            return self.styles_in_per_cell(sheet, range, out);
        }
        // A transient throw is a fall-back, not a corruption.
        let jsv = match self.note_throw(
            "getCellStylesIn",
            self.handle
                .get_cell_styles_in(sheet, range.r1, range.c1, range.r2, range.c2),
        ) {
            Some(v) => v,
            None => return self.styles_in_per_cell(sheet, range, out),
        };
        // The array element shape is the per-cell shape — no new wire struct.
        let decoded: Vec<Option<ic::Style>> = match serde_wasm_bindgen::from_value(jsv) {
            Ok(v) => v,
            Err(e) => {
                self.note_serde_err("getCellStylesIn", &e);
                return self.styles_in_per_cell(sheet, range, out);
            }
        };

        // D-2: only a wrong-length array is untrustworthy now; a null element
        // is a blank cell. A *clean dense batch* never carries `BridgeFailed`:
        // the whole-batch throw is the only failure here and routed to per-cell
        // above — so a null element is `Absent`, not a transient failure.
        if !batch_is_dense(&decoded, range) {
            return self.styles_in_per_cell(sheet, range, out);
        }
        out.clear();
        // One theme borrow for the whole batch — not one cache hit per cell.
        self.with_theme(|t| {
            out.extend(decoded.into_iter().map(|s| match s {
                Some(s) => Fetched::Value(ic_style_to_core(s, t)),
                None => Fetched::Absent,
            }));
        });
    }

    fn get_formatted_cell_values_in(
        &self,
        sheet: u32,
        range: RCRange,
        out: &mut Vec<Fetched<String>>,
    ) {
        if !self.has_values_in {
            return self.values_in_per_cell(sheet, range, out);
        }
        let jsv = match self.note_throw(
            "getFormattedCellValuesIn",
            self.handle
                .get_formatted_cell_values_in(sheet, range.r1, range.c1, range.r2, range.c2),
        ) {
            Some(v) => v,
            None => return self.values_in_per_cell(sheet, range, out),
        };
        let decoded: Vec<Option<String>> = match serde_wasm_bindgen::from_value(jsv) {
            Ok(v) => v,
            Err(e) => {
                self.note_serde_err("getFormattedCellValuesIn", &e);
                return self.values_in_per_cell(sheet, range, out);
            }
        };
        if !batch_is_dense(&decoded, range) {
            return self.values_in_per_cell(sheet, range, out);
        }
        out.clear();
        out.extend(decoded.into_iter().map(|v| match v {
            Some(v) => Fetched::Value(v),
            None => Fetched::Absent,
        }));
    }

    fn get_cell_types_in(&self, sheet: u32, range: RCRange, out: &mut Vec<Fetched<CellKind>>) {
        if !self.has_types_in {
            return self.types_in_per_cell(sheet, range, out);
        }
        let jsv = match self.note_throw(
            "getCellTypesIn",
            self.handle
                .get_cell_types_in(sheet, range.r1, range.c1, range.r2, range.c2),
        ) {
            Some(v) => v,
            None => return self.types_in_per_cell(sheet, range, out),
        };
        let decoded: Vec<Option<i32>> = match serde_wasm_bindgen::from_value(jsv) {
            Ok(v) => v,
            Err(e) => {
                self.note_serde_err("getCellTypesIn", &e);
                return self.types_in_per_cell(sheet, range, out);
            }
        };
        if !batch_is_dense(&decoded, range) {
            return self.types_in_per_cell(sheet, range, out);
        }
        // A valid discriminant maps to a `CellKind`; a null or out-of-range one
        // yields `Absent` — the same legitimate per-cell outcome (matching
        // single-cell `get_cell_type`), not a corruption.
        out.clear();
        out.extend(
            decoded
                .into_iter()
                .map(|d| match d.and_then(cell_kind_from_discriminant) {
                    Some(k) => Fetched::Value(k),
                    None => Fetched::Absent,
                }),
        );
    }
}

/// Whether a batched `*_in` array can be trusted enough to skip the per-cell
/// path: it must be dense, exactly the range's cell count, so every
/// row-major index `out[(r-r1)*cols + (c-c1)]` lands on a real slot.
///
/// A `None` *element* is no longer a reason to forfeit the batch. The bulk
/// contract treats a null slot as a blank cell (`Absent`), not a failure: the
/// renderer paints it over the pre-filled `cell_bg`, so a pane full of blanks
/// renders correctly without the O(cells) per-cell refetch the old all-`Some`
/// gate forced. The genuine failure modes — a throw, a non-array payload, or a
/// wrong-length array — are caught upstream (the `note_throw` / `note_serde_err`
/// fallback arms) or by this length check, and route to the per-cell path.
fn batch_is_dense<T>(decoded: &[Option<T>], range: RCRange) -> bool {
    let rows = (range.r2 - range.r1 + 1) as usize;
    let cols = (range.c2 - range.c1 + 1) as usize;
    decoded.len() == rows * cols
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

/// Convert an IronCalc `Style` (deserialized from JS) to the core `CellStyle`,
/// resolving theme colors against the workbook theme.
/// Mirrors `iron-canvas-ironcalc::convert::style_to_core` — kept local to
/// avoid pulling `iron-canvas-ironcalc` into the web crate's dep tree.
fn ic_style_to_core(s: ic::Style, theme: &ic::Theme) -> CellStyle {
    CellStyle {
        fill_color: ic_color_to_css(&s.fill.color, theme),
        font: FontStyle {
            name: s.font.name,
            size: f64::from(s.font.sz),
            color: ic_color_to_css(&s.font.color, theme),
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
            left: s.border.left.map(|i| ic_border_item_to_core(i, theme)),
            right: s.border.right.map(|i| ic_border_item_to_core(i, theme)),
            top: s.border.top.map(|i| ic_border_item_to_core(i, theme)),
            bottom: s.border.bottom.map(|i| ic_border_item_to_core(i, theme)),
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

fn ic_border_item_to_core(b: ic::BorderItem, theme: &ic::Theme) -> BorderItem {
    BorderItem {
        style: ic_border_style_to_core(b.style),
        color: ic_color_to_css(&b.color, theme),
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
    fn batch_is_dense_accepts_null_blank_cells() {
        // The Phase-3 contract change: a null element is a blank cell, not a
        // failure, so an otherwise correctly-sized array is still trusted —
        // a blank-cell pane no longer forces an O(cells) per-cell refetch.
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        let decoded: Vec<Option<i32>> = vec![Some(1), None, None, Some(4)];
        assert!(batch_is_dense(&decoded, range));
    }

    #[test]
    fn batch_is_dense_rejects_wrong_length() {
        // 2×2 range ⇒ 4 cells. Anything else can't be indexed row-major, so it
        // forfeits the batch and falls back to the per-cell path.
        let range = RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        let short: Vec<Option<i32>> = vec![Some(1), Some(2), Some(3)];
        let long: Vec<Option<i32>> = vec![None; 5];
        assert!(!batch_is_dense(&short, range));
        assert!(!batch_is_dense(&long, range));
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

    #[test]
    fn mirror_resolves_theme_colors() {
        let theme = ic::Theme::default();
        let s = ic::Style {
            fill: ic::Fill {
                color: ic::Color::Theme(4, 0.0), // accent1
            },
            ..ic::Style::default()
        };
        let core = ic_style_to_core(s, &theme);
        assert_eq!(core.fill_color.as_deref(), Some("#4472C4"));
        assert_eq!(
            ic_color_to_css(&ic::Color::None, &theme),
            None,
            "Color::None must stay None, not become an empty CSS string"
        );
    }
}

pub mod diag;
