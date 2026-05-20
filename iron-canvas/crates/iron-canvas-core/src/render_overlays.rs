//! Overlay state input.
//!
//! Selection is not stored here — it is paint-time-derived from
//! `model.get_selected_view()`. The web crate signals selection changes via
//! `IronCanvas::request_overlay_repaint()`.

use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};

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
