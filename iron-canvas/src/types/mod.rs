//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component
//!
//! `*Paint` submodules hold renderer-ready snapshots resolved from the model.
//! Convention: resolve in `crate::types`, paint in `crate::renderer`.

pub(crate) mod text_paint;

use crate::model::{FormulaRef, RCRange, SheetArea};
use crate::renderer::AutofillTarget;

/// Overlay ranges passed to `render()` for selection preview drawing.
#[derive(Clone, PartialEq, Default)]
pub struct RenderOverlays {
    /// Selection border in CSS pixels, pre-converted by the consumer.
    /// `None` means no selection is visible (e.g., during a sheet swap).
    pub selection: Option<super::geometry::PixelRect>,
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
}
