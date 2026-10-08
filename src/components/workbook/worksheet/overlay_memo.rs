//! Reactive overlay Memo — derives the renderer-facing `OverlayState` from
//! the model's selected view, drag state and editing state.
//!
//! Lives in a memo, not a direct Effect subscription: if the subscribe
//! Effect read drag/editing state directly, `set_drag(Selecting)` in
//! `on_mousedown` would cause an extra Effect run (and an extra render)
//! before the navigation event fires. The memo's `PartialEq` gate also
//! suppresses spurious renders: `Selecting` and `Idle` both map to the same
//! selection, so switching between them doesn't change the memo output.
//!
//! The clipboard is NOT in this memo because it lives in a `StoredValue`
//! (non-reactive). The rAF loop patches it into the request each frame so it
//! never goes stale (the original marching-ants bug).

use iron_canvas::{FormulaOverlay, OverlayState};
use iron_canvas_core::CellCoord;
use leptos::prelude::*;

use crate::coord::CellArea;
use crate::input::mouse::{preview_fill_extension, resolved_fill_target};
use crate::state::{DragState, ModelStore, WorkbookState};

pub(super) fn reactive_overlay(state: WorkbookState, model: ModelStore) -> Memo<OverlayState> {
    Memo::new(move |_| {
        // Subscribe to the navigation bus so a selection/sheet move re-derives
        // the selection overlay; the model store itself is not reactive.
        let _ = state.events.navigation.get();

        let mut overlays = OverlayState::default();
        let view = model.with_value(|m| m.get_selected_view());
        let source = CellArea::from(view.range).normalized();
        overlays.selection.push(source.into());
        if let DragState::Extending { to_row, to_col } = state.drag.get() {
            let target = resolved_fill_target(model, to_row, to_col);
            if let Some(extension) = preview_fill_extension(source, target) {
                overlays.selection.push(extension.into());
            }
        }
        overlays.active_cell = Some(CellCoord {
            row: view.row,
            col: view.column,
        });

        // Reading editing_cell here subscribes the memo to it. Since the
        // formula analysis derives PartialEq, the memo's PartialEq gate
        // suppresses re-renders when refs don't change (e.g. text changed but
        // no new refs produced).
        let editing_cell = state.editing_cell.get();
        if let Some(edit) = editing_cell.as_ref() {
            let dragged_ref = match state.drag.get() {
                DragState::DraggingFormulaRef {
                    ref_idx, preview, ..
                } => Some((ref_idx, preview)),
                _ => None,
            };
            for (ref_idx, reference) in edit.formula_analysis.refs().iter().enumerate() {
                let area = dragged_ref
                    .filter(|(dragged_idx, _)| *dragged_idx == ref_idx)
                    .map_or(reference.sheet_area, |(_, preview)| preview);
                overlays.formula_references.push(FormulaOverlay {
                    sheet: area.sheet,
                    range: area.area.into(),
                    color_index: reference.color_idx,
                });
            }
        }

        // Point-mode range for overlay painting. RefNode stores relative
        // deltas, so resolution needs the editing cell's address as anchor.
        // Cross-sheet pointing: the canvas only shows the selected sheet, so
        // the rectangle is suppressed when the pointed sheet isn't visible.
        if let (DragState::Pointing { ref_node, .. }, Some(edit)) =
            (state.drag.get(), editing_cell.as_ref())
        {
            let range = ref_node.area(&edit.address);
            if range.sheet == view.sheet {
                let color_index = overlays.formula_references.len();
                overlays.formula_references.push(FormulaOverlay {
                    sheet: range.sheet,
                    range: range.area.into(),
                    color_index,
                });
            }
        }

        overlays
    })
}
