//! `iron-canvas-core` — the pure-Rust application layer of the iron-canvas
//! grid renderer. Browser bindings live in sibling `iron-canvas-web`.
//!
//! See `crate::painter::Painter` for the drawing surface, `crate::chrome::Chrome`
//! for the per-frame snapshot, and `crate::renderer` for the paint passes.

pub mod chrome;
pub mod decoration;
pub mod geometry;
pub mod layer;
pub mod model_adapter;
mod orchestrator;
pub mod painter;
mod render_overlays;
pub mod renderer;
pub mod signal;
pub mod theme;
pub mod types;

pub use orchestrator::{Orchestrator, PaintRegime, PaintRegimeTag};

pub use render_overlays::RenderOverlays;

pub use geometry::{
    CanvasSize,
    constants::{
        AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
        HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::{Line, Point, Span},
    utils::col_name,
};
pub use model_adapter::{CanvasModel, CanvasView};
pub use theme::{CanvasTheme, ThemeVariables};
pub use types::coord::{AutofillTarget, FormulaRef, FormulaRefKind, RCRange, SheetArea};
pub use types::ui::{Corner, HitTest, RefZone, ResizeTarget, Side};
