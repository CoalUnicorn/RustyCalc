//! Wasm-bound facade for the iron-canvas grid renderer.
//!
//! The pure-Rust application and domain layers live in `iron-canvas-core`;
//! the Canvas2D `Painter` impl, `WebSurface` adapter, paired runtime, and
//! CSS-var theme bridge live in `iron-canvas-canvas2d` (re-exported below).
//! This crate
//! owns the `wasm-bindgen` `IronCanvas` handle, the JS-bridged
//! `JsBackedModel`, and the dev-tools recording / playback glue.
//!
//! Everything `iron-canvas-core` and `iron-canvas-canvas2d` re-export flows
//! through here unchanged, so downstream call sites name a single facade crate.

mod orchestrator;
#[cfg(feature = "dev-tools")]
mod playback;
#[cfg(feature = "dev-tools")]
mod replay;
pub mod wasm;
#[cfg(any(target_arch = "wasm32", feature = "dev-tools"))]
mod wire;

pub use iron_canvas_canvas2d::{Canvas2dRuntime, CanvasPainter, WebSurface, theme_from_element};
pub use iron_canvas_core::geometry::utils::col_name;
pub use iron_canvas_core::{
    AUTOFILL_HANDLE_PX, AutofillTarget, CanvasModel, CanvasSize, CanvasTheme, CanvasView,
    CellContentQuery, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, FormulaRef,
    FormulaRefKind, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT, HEADER_SEPARATOR_WIDTH, HitTest,
    LAST_COLUMN, LAST_ROW, Line, PixelRect, Point, RCRange, RectCorner, RefZone, RenderOverlays,
    ResizeTarget, SheetArea, Side, Span, ThemeVariables, chrome, decoration, geometry,
    model_adapter, painter, renderer, theme, types,
};
pub use orchestrator::{IronCanvas, RenderResult};
