//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component
//!
//! `*Paint` submodules hold renderer-ready snapshots resolved from the model.
//! Convention: resolve in `crate::types`, paint in `crate::renderer`.

pub mod coord;
pub(crate) mod text_paint;
pub mod ui;

use crate::renderer::AutofillTarget;
use coord::{FormulaRef, RCRange, SheetArea};

/// Overlay ranges passed to `render()`.
///
/// Selection is not stored here — it is paint-time-derived from
/// `model.get_selected_view()`. The consumer signals selection changes via
/// `IronCanvas::request_overlay_repaint()`.
#[derive(Clone, PartialEq, Default)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
}
