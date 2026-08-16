//! `iron-canvas-core` — the pure-Rust application layer of the iron-canvas
//! grid renderer. Browser bindings live in sibling `iron-canvas-web`.
//!
//! See [`Painter`](crate::painter::Painter) for the drawing surface,
//! [`Chrome`](crate::chrome::Chrome) for the per-frame snapshot, and
//! [`renderer`](crate::renderer) for the paint passes.

pub mod autofit;
pub mod chrome;
pub mod decoration;
mod frame_plan;
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

pub use frame_plan::{FrameDelta, FrameInputFailure, FrameInputs, RebuildReason};
pub use orchestrator::{
    FrameOutcome, FrameTrace, GridVerdict, Orchestrator, PaintRegimeTag, PaintResult,
};

#[cfg(feature = "dev-diagnostics")]
pub use renderer::diag::{
    DiagBlit, DiagBlitResultTag, DiagBufferTruth, DiagCache, DiagCacheActionTag,
    DiagCacheResolution, DiagCacheTruth, DiagDeltaKind, DiagFetch, DiagFetchPurpose,
    DiagFetchRequest, DiagFingerprintActionTag, DiagFingerprintTruth, DiagGeometry,
    DiagPaintCounts, DiagPaintedLayers, DiagRepaint, DiagRepaintReason, DiagRevealedStrip,
    DiagSegment, FrameDiagnostics,
};

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
