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
//! - [`geometry`] — rect/line primitives and pixel↔cell coordinate math.
//! - [`renderer`] — `RendererCore` plus the `cell/` and `chrome/` paint
//!   subtrees. Overlay decorations (selection, autofill, clipboard,
//!   point-mode, formula refs) live in `src/layer/decoration/` as `Layer`
//!   trait impls, orchestrated by `OverlayLayer::paint`. See the module
//!   doc for the pipeline walk-through.
//! - [`theme`] — `CanvasTheme` palette + CSS-var bridge.
//! - [`types`] — public address types (`RCRange`, `FormulaRef`) and UI
//!   variants (`HitTest`, `ResizeTarget`) used at the JS surface.
//! - [`wasm`] — `JsBackedModel` JS-side adapter.

pub mod geometry;

mod chrome;
mod layer;
mod model_adapter;
mod orchestrator;
mod painter;
pub mod renderer;
pub(crate) mod signal;
pub mod theme;
pub mod types;
pub mod wasm;

#[cfg(test)]
mod test;

pub use geometry::{
    constants::{
        AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
        HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::{Line, Point, Span},
    utils::col_name,
    CanvasSize,
};

pub use layer::RenderOverlays;
pub use model_adapter::{CanvasModel, CanvasView};
pub use orchestrator::IronCanvas;
pub use theme::{CanvasTheme, ThemeVariables};
pub use types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
pub use types::ui::{HitTest, ResizeTarget};
