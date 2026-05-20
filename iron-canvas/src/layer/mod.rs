//! Two-canvas layering glue (web crate).
//!
//! `LayerBase`, `PaintGate`, and `Surface` live in `iron-canvas-core`. This
//! module hosts the wasm-bound `GridLayer` / `OverlayLayer` specializations
//! that pair a `WebSurface` with the layer-specific renderer.

mod grid;
mod overlay;

pub(crate) use grid::GridLayer;
pub(crate) use iron_canvas_core::decoration::{
    autofill::AutofillLayer, clipboard::ClipboardLayer, formula_refs::FormulaRefsLayer,
    point_mode::PointModeLayer, selection::SelectionLayer, Layer,
};
pub(crate) use overlay::OverlayLayer;
pub use overlay::RenderOverlays;
