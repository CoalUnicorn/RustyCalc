//! `iron-canvas-core` — the pure-Rust application layer of the iron-canvas
//! grid renderer. Browser bindings live in sibling `iron-canvas-web`.
//!
//! See [`Painter`](crate::painter::Painter) for the drawing surface,
//! [`Chrome`](crate::chrome::Chrome) for the per-frame snapshot, and
//! [`renderer`](crate::renderer) for the paint passes.

pub mod autofit;
pub mod chrome;
pub mod decoration;
pub mod geometry;
pub mod layer;
pub mod model_adapter;
mod orchestrator;
pub mod painter;
mod pending_work;
mod render_overlays;
pub mod renderer;
mod style;
pub mod theme;
pub mod types;

pub use orchestrator::{
    FrameOutcome, FrameTrace, Orchestrator, PaintRegime, PaintRegimeTag, PaintResult, PaneVerdict,
};

pub use renderer::blit_work::{BlitPaneWork, widen_blit_strip_to_pixel_clip};
pub use renderer::cache::{PaneBlitAddressWork, PaneShiftPrep};

pub use render_overlays::RenderOverlays;

pub use decoration::{DecorationId, Layer};
pub use geometry::{
    CanvasSize,
    constants::{
        AUTOFILL_HANDLE_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
        HEADER_ROW_HEIGHT, HEADER_SEPARATOR_WIDTH, LAST_COLUMN, LAST_ROW,
    },
    pixel_rect::PixelRect,
    prim::{Line, Point, Span},
    utils::col_name,
};
pub use model_adapter::{CanvasModel, CanvasView, CellContentQuery};
pub use pending_work::{RowSpan, WorkFlags};
pub use style::{
    Alignment, Border, BorderItem, BorderStyle, CellDecoration, CellKind, CellStyle, DataBarSpec,
    FontStyle, HAlign, IconSpec, RatingSpec, VAlign,
};
pub use theme::{CanvasTheme, ThemeVariables};
pub use types::coord::{AutofillTarget, FormulaRef, FormulaRefKind, RCRange, SheetArea};
pub use types::fetched::Fetched;
pub use types::ui::{HitTest, RectCorner, RefZone, ResizeTarget, Side};
