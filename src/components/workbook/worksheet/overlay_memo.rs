//! Reactive overlay Memo — derives the renderer-facing overlay payload
//! from drag state, editing state, and the dragged-ref override.
//!
//! Lives in a memo, not a direct Effect subscription: if the subscribe
//! Effect read drag/point_range directly, `set_drag(Selecting)` in
//! `on_mousedown` would cause an extra Effect run (and an extra render)
//! before the navigation event fires. The memo's `PartialEq` gate also
//! suppresses spurious renders: `Selecting` and `Idle` both map to
//! `extend_to=None`, so switching between them doesn't change the memo
//! output and doesn't re-render.
//!
//! The clipboard is NOT in this memo because it lives in a `StoredValue`
//! (non-reactive). It is read fresh in the rAF callback each render so it
//! never goes stale (the original marching-ants bug).

use iron_canvas_core::types::coord::AutofillTarget;
use leptos::prelude::*;

use crate::coord::{ActiveRef, CellArea};
use crate::state::{DragState, WorkbookState};

/// Named so the subscribe-Effect's `prev: Option<OverlayTuple>` reads
/// cleanly and the `PartialEq` gate is explicit instead of relying on
/// Rust's tuple-equality blanket impl.
#[derive(Clone, PartialEq)]
pub(super) struct OverlayTuple {
    pub extend_to: Option<AutofillTarget>,
    pub point_range: Option<CellArea>,
    pub formula_refs: Vec<ActiveRef>,
}

pub(super) fn reactive_overlay(state: WorkbookState) -> Memo<OverlayTuple> {
    Memo::new(move |_| {
        let extend_to = if let DragState::Extending { to_row, to_col } = state.drag.get() {
            Some(AutofillTarget {
                row: to_row,
                col: to_col,
            })
        } else {
            None
        };

        // Reading editing_cell here subscribes the memo to it. Since FormulaRef
        // derives PartialEq, the memo's PartialEq gate suppresses re-renders
        // when refs don't change (e.g. text changed but no new refs produced).
        let editing_cell = state.editing_cell.get();
        let mut formula_refs: Vec<ActiveRef> = editing_cell
            .as_ref()
            .map(|e| e.formula_analysis.refs().to_vec())
            .unwrap_or_default();

        // Live drag ghost: while `DraggingFormulaRef` is active, mousemove
        // publishes a `RefOverride`; we patch the matching ref's `sheet_area`
        // so the painted outline follows the cursor without touching the
        // formula text. Bounds-checked: if the formula was re-analyzed
        // mid-drag and refs shrank, the patch is silently skipped.
        if let Some(o) = state.dragged_ref_override.get()
            && let Some(r) = formula_refs.get_mut(o.idx)
        {
            r.sheet_area = o.range;
        }

        // Point-mode range for overlay painting. RefNode stores relative
        // deltas, so resolution needs the editing cell's address as anchor.
        let point_range = match (state.drag.get(), editing_cell.as_ref()) {
            (DragState::Pointing { ref_node, .. }, Some(e)) => Some(ref_node.area(&e.address).area),
            _ => None,
        };

        OverlayTuple {
            extend_to,
            point_range,
            formula_refs,
        }
    })
}
