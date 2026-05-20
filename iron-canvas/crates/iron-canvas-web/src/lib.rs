//! The spreadsheet's `<canvas>` surface — wasm-bound facade.
//!
//! Pure-Rust application + domain layers live in `iron-canvas-core`. This
//! crate hosts the Canvas2D `Painter` impl, the wasm-bindgen `IronCanvas`
//! handle, the JS-bridged `JsBackedModel`, the `WebSurface` adapter, and
//! the CSS-var theme bridge — everything that touches `web-sys` /
//! `wasm-bindgen` / `js-sys`.
//!
//! Everything `iron-canvas-core` re-exports flows through here unchanged
//! so downstream call sites can name a single facade crate.

mod canvas_painter;
mod layer;
mod orchestrator;
pub mod theme_from_element;
pub mod wasm;
pub mod web_surface;

#[cfg(test)]
mod test;

pub use iron_canvas_core::geometry::utils::col_name;
pub use iron_canvas_core::{
    chrome, decoration, geometry, model_adapter, painter, renderer, signal, theme, types,
    AutofillTarget, CanvasModel, CanvasSize, CanvasTheme, CanvasView, FormulaRef, HitTest, Line,
    PixelRect, Point, RCRange, ResizeTarget, SheetArea, Span, ThemeVariables, AUTOFILL_HANDLE_PX,
    DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_OFFSET,
    HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
};
pub use layer::RenderOverlays;
pub use orchestrator::IronCanvas;
