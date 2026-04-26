//! The spreadsheet's `<canvas>` surface.
//!
//! # The Concept - everything is a rectangle or a line
//!
//! Every visible artifact in the grid is one of two primitives:
//!
//! - [`PixelRect`] - cells, headers, corner box, selection fill, autofill
//!   handle, point-mode tint, clipboard marching-ants region, text clip.
//! - [`Line`] - border edges, frozen-pane separators, underline,
//!   strikethrough.
//!
//! No curves, no arbitrary paths. Border resolution becomes "pick a `Line`
//! and a color"; pane layout becomes "four `PixelRect`s side by side";
//! overlays compose by stacking `PixelRect`s.
//!
//! That constraint keeps the paint layer small: `rect_fill`, `rect_stroke`,
//! `rect_dashed`, `stroke_line`, `with_clip`. New visuals reduce to those
//! helpers or they don't ship.
//!
//! # Submodules
//!
//! - [`geometry`] - rect/line types and pixel↔cell coordinate math.
//! - [`types`] - renderer-internal shapes (panes, text layout, visible
//!   region) plus public overlay types.
//! - [`renderer`] - the four-phase render pipeline. See its module doc
//!   for the full walk-through.

pub mod geometry;
pub mod model;
pub mod renderer;
pub mod style;
pub mod theme;
pub mod types;

pub use geometry::*;
pub use renderer::CanvasRenderer;
pub use types::{CanvasRenderMode, RenderOverlays};

// CanvasModel - read-only worksheet surface the renderer consumes
//
// Path A bridge: the renderer's eventual parameter type. Replaces direct
// `&UserModel` so the IronCalc webapp can plug in a JS-backed adapter that
// batch-fetches cell data into a frame snapshot before each `render_sheet()`.
//
// Method signatures match `ironcalc_base::UserModel` verbatim (T1-A) in a
// single trait (T2-A). RustyCalc gets a free `impl CanvasModel for UserModel`
// below.
//
// `SelectedView` is mirrored locally because `ironcalc_base` re-exports the
// upstream struct only under `#[cfg(test)]`. When that gate goes away
// upstream, this mirror and the field-copy in `get_selected_view` below can
// be deleted in favour of `pub use ironcalc_base::SelectedView;` - no other
// consumer needs to change.

use ironcalc_base::types::{CellType, Style};
use ironcalc_base::UserModel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedView {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
    pub range: [i32; 4],
    pub top_row: i32,
    pub left_column: i32,
}

pub trait CanvasModel {
    fn get_selected_sheet(&self) -> u32;
    fn get_selected_view(&self) -> SelectedView;
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String>;
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String>;
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String>;
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String>;
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String>;
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String>;
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32)
        -> Result<String, String>;
}

impl<'a> CanvasModel for UserModel<'a> {
    fn get_selected_sheet(&self) -> u32 {
        UserModel::get_selected_sheet(self)
    }
    fn get_selected_view(&self) -> SelectedView {
        let v = UserModel::get_selected_view(self);
        SelectedView {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
            range: v.range,
            top_row: v.top_row,
            left_column: v.left_column,
        }
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_rows_count(self, sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Result<i32, String> {
        UserModel::get_frozen_columns_count(self, sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Result<f64, String> {
        UserModel::get_row_height(self, sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Result<f64, String> {
        UserModel::get_column_width(self, sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Result<bool, String> {
        UserModel::get_show_grid_lines(self, sheet)
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Result<Style, String> {
        UserModel::get_cell_style(self, sheet, row, column)
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Result<CellType, String> {
        UserModel::get_cell_type(self, sheet, row, column)
    }
    fn get_formatted_cell_value(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<String, String> {
        UserModel::get_formatted_cell_value(self, sheet, row, column)
    }
}

// WASM host surface - IronCanvasView
//
// Drop-in replacement for IronCalc's
// `webapp/IronCalc/src/components/WorksheetCanvas/worksheetCanvas.ts`.
// `CanvasRenderer` stays stateless per frame; `IronCanvasView` owns the
// long-lived inputs (canvas element, theme, scroll state, callbacks) and
// builds a fresh renderer inside each `render_sheet()` call.
//
// Architectural decisions in force:
//
//   Fork 1 - Model handle: shared `UserModel`. Settings receive the ironcalc
//   `Model` JS handle and we read it directly via wasm-bindgen. Bridge
//   mechanism (extern type + `unchecked_ref` cast vs ironcalc-side getter
//   that exposes the inner `&UserModel`) is wired in a follow-up pass.
//
//   Fork 2 - Overlays: canvas-painted (renderer Phase 3). IronCalc's
//   `cellOutline` / `areaOutline` / `extendToOutline` / `cellArrayStructure`
//   divs are no longer needed.
//     Option B reserved - drive DOM divs by absolute positioning. Would add
//     `update_cell_outline(rect)`, `update_area_outline(rect)`,
//     `update_extend_to_outline(rect)`, plus a `disable_internal_overlays()`
//     toggle that skips Phase 3 inside `CanvasRenderer::render`. Not in v1.
//
//   Fork 3 - Theme: read CSS variables off the canvas's closest `.ic-root`
//   ancestor at construction (`CanvasTheme::from_css_vars(&root)` - to be
//   added on `CanvasTheme`). Caller does not pass colors.
//     Option B reserved - accept a serialized `CanvasTheme` from JS via
//     `set_theme(theme: JsValue)`. Useful for hosts that compute theme
//     outside CSS variables. Not in v1.

use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
use web_sys::OffscreenCanvas;

#[allow(dead_code, unused)]
#[wasm_bindgen]
pub struct IronCanvasView {
    canvas: HtmlCanvasElement,
    width: f64,
    height: f64,
    scroll_left: f64,
    scroll_top: f64,
    model: JsValue,
    on_column_width_changes: js_sys::Function,
    on_row_height_changes: js_sys::Function,
    refresh: js_sys::Function,
}

#[wasm_bindgen]
impl IronCanvasView {
    #[wasm_bindgen(constructor)]
    pub fn new(
        canvas: HtmlCanvasElement,
        width: f64,
        height: f64,
        model: JsValue,
        on_column_width_changes: js_sys::Function,
        on_row_height_changes: js_sys::Function,
        refresh: js_sys::Function,
    ) -> Self {
        Self {
            canvas,
            width,
            height,
            scroll_left: 0.0,
            scroll_top: 0.0,
            model,
            on_column_width_changes,
            on_row_height_changes,
            refresh,
        }
    }

    /// Full re-paint of the visible sheet.
    pub fn render_sheet(&mut self) {
        // Pending: borrow `&UserModel` from `self.model`, build
        // `CanvasRenderer::new(&self.canvas, theme, dpr)`, assemble a
        // `RenderOverlays` from current selection / autofill / clipboard
        // state, dispatch the four-phase render.
    }

    /// Update scroll origin (CSS pixels) and request a repaint.
    pub fn set_scroll_position(&mut self, left: f64, top: f64) {
        self.scroll_left = left;
        self.scroll_top = top;
    }

    /// Match the backing element after a layout change.
    pub fn resize(&mut self, width: f64, height: f64) {
        self.width = width;
        self.height = height;
    }

    // Reserved for Fork 2 (Option B - DOM-driven overlays).
    // pub fn update_cell_outline(&self, _rect: JsValue) { /* unimplemented */ }
    // pub fn update_area_outline(&self, _rect: JsValue) { /* unimplemented */ }
    // pub fn update_extend_to_outline(&self, _rect: JsValue) { /* unimplemented */ }
    // pub fn disable_internal_overlays(&mut self, _disabled: bool) { /* unimplemented */ }

    // Reserved for Fork 3 (Option B - caller-provided theme).
    // pub fn set_theme(&mut self, _theme: JsValue) { /* unimplemented */ }

    // Hit-test surface. Both shapes ship:
    //   - Split methods (cheap, single-purpose) for hot-path callers like
    //     `mousemove` cursor flips that don't want a JS object.
    //   - `hit_test` (rich, single-call) for `pointerdown` dispatch that
    //     wants one exhaustive switch on the JS side.
    #[allow(dead_code, unused)]
    pub fn get_cell_by_coordinates(&self, x: f64, y: f64) -> JsValue {
        todo!()
    }

    #[allow(dead_code, unused)]
    /// Returns `{ axis, index }` or null.
    pub fn get_header_by_coordinates(&self, x: f64, y: f64) -> JsValue {
        todo!()
    }

    #[allow(dead_code, unused)]
    pub fn is_on_autofill_handle(&self, x: f64, y: f64) -> bool {
        todo!()
    }

    #[allow(dead_code, unused)]
    pub fn hit_test(&self, x: f64, y: f64) -> JsValue {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub enum HitTest {
    Cell { sheet: u32, row: i32, column: i32 },
    RowHeader(u32),
    ColHeader(u32),
    Corner,
    AutofillHandle,
    Outside,
}
