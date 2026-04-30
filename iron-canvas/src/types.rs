//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

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

/// Scroll origin for the visible sheet area.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Viewport {
    /// First visible row in the scrollable region (1-indexed).
    pub top_row: i32,
    /// First visible column in the scrollable region (1-indexed).
    pub left_column: i32,
}

/// Number of rows/columns pinned by the freeze-panes feature.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FreezeConfig {
    pub frozen_rows: u32,
    pub frozen_cols: u32,
}
