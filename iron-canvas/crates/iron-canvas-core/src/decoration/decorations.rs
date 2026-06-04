//! The five overlay decoration layers as one owned group.
//!
//! Before this struct existed the orchestrator held `selection`,
//! `autofill`, `clipboard`, `point_mode`, and `formula_refs` as five loose
//! fields. They were always touched together — painted as a slice,
//! hit-tested as an array, refreshed in lockstep — yet every paint arm
//! re-wrote the same slice literal and every mutator re-implemented the
//! compare-then-raise dance. `Decorations` makes the group a type:
//!
//! - Paint and hit-test order live in exactly one method each
//!   (`overlay_slice` / `hit_order`), not copied across four paint sites.
//! - Mutation is only reachable through the `set_*` methods, each of which
//!   reports whether anything changed. The caller raises `OVERLAY` on
//!   `true`; silently poking a field and forgetting the raise is no longer
//!   expressible, because the fields are private to this module.
//! - The selection→autofill mirror that keeps the drag preview
//!   paint-coherent is owned by `refresh_overlay_state`, the single source
//!   of that invariant.

use crate::CanvasModel;
use crate::render_overlays::RenderOverlays;
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};

use super::{
    AutofillLayer, ClipboardLayer, FormulaRefsLayer, Layer, PointModeLayer, RepaintActiveCell,
    SelectionLayer,
};

#[derive(Default)]
pub(crate) struct Decorations {
    selection: SelectionLayer,
    autofill: AutofillLayer,
    clipboard: ClipboardLayer,
    point_mode: PointModeLayer,
    formula_refs: FormulaRefsLayer,
}

impl Decorations {
    pub(crate) fn selection(&self) -> &SelectionLayer {
        &self.selection
    }

    /// Active-cell repaint coords, fired between the selection fill and
    /// stroke phases. Exposed so the orchestrator can gate
    /// `CONTENT ⇒ OVERLAY` (a DEL on the active cell) without reaching past
    /// the group into the selection field.
    pub(crate) fn active_cell_repaint(&self) -> Option<RepaintActiveCell> {
        self.selection.active_cell_repaint()
    }

    /// Back-to-front paint order for `OverlayRenderer::paint_overlay_layer`.
    /// Selection is *not* in this slice — it drives the three-phase
    /// fill → active-cell repaint → stroke sequence and is passed
    /// separately to the renderer.
    pub(crate) fn overlay_slice(&self) -> [&dyn Layer; 4] {
        [
            &self.autofill,
            &self.clipboard,
            &self.point_mode,
            &self.formula_refs,
        ]
    }

    /// Front-to-back hit-test priority: formula refs win over point mode
    /// over clipboard over autofill over selection. The one place this
    /// order is written.
    pub(crate) fn hit_order(&self) -> [&dyn Layer; 5] {
        [
            &self.formula_refs,
            &self.point_mode,
            &self.clipboard,
            &self.autofill,
            &self.selection,
        ]
    }

    /// Refresh selection from the live model, then mirror its rectangle
    /// into the autofill preview so the drag band stays paint-coherent
    /// with the painted selection instead of chasing the model. Called at
    /// the top of every paint regime arm.
    pub(crate) fn refresh_overlay_state(&mut self, model: &dyn CanvasModel) {
        self.selection.refresh(model);
        self.autofill.selection_range = self.selection.selection_range.unwrap_or_default();
    }

    /// Bulk compare-and-set the four overlay inputs from a `RenderOverlays`
    /// bag. Returns `true` if any field changed; the caller raises
    /// `OVERLAY` on `true`. Folding the four comparisons into one pass lets
    /// the Leptos host's per-frame memo cost a single raise, not four.
    pub(crate) fn set_overlays(&mut self, overlays: RenderOverlays) -> bool {
        let RenderOverlays {
            extend_to,
            clipboard,
            point_range,
            formula_refs,
        } = overlays;
        self.set_extend_to(extend_to)
            | self.set_clipboard(clipboard)
            | self.set_point_range(point_range)
            | self.set_formula_refs(formula_refs)
    }

    pub(crate) fn set_extend_to(&mut self, target: Option<AutofillTarget>) -> bool {
        let changed = self.autofill.extend_to != target;
        if changed {
            self.autofill.extend_to = target;
        }
        changed
    }

    pub(crate) fn set_clipboard(&mut self, area: Option<SheetArea>) -> bool {
        let changed = self.clipboard.clipboard != area;
        if changed {
            self.clipboard.clipboard = area;
        }
        changed
    }

    pub(crate) fn set_point_range(&mut self, range: Option<RCRange>) -> bool {
        let changed = self.point_mode.point_range != range;
        if changed {
            self.point_mode.point_range = range;
        }
        changed
    }

    pub(crate) fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) -> bool {
        let changed = self.formula_refs.refs != refs;
        if changed {
            self.formula_refs.refs = refs;
        }
        changed
    }
}
