//! Formula-reference drag sub-grammar.
//!
//! When the user grabs a coloured reference rectangle on the canvas
//! during point-mode (formula editing), this module owns the drag
//! lifecycle: start, range computation each mousemove, commit on
//! mouseup. The dragged ref splicing back into the formula buffer
//! lives in `crate::input::formula::splice_dragged_ref`.

use leptos::prelude::*;

use crate::coord::{CellAddress, SheetRange};
use crate::input::formula::splice_dragged_ref;
use crate::model::FormulaAnalyzer;
use crate::state::{DragState, ModelStore, RefOverride, WorkbookState};
use iron_canvas_core::geometry::constants::{LAST_COLUMN, LAST_ROW};
use iron_canvas_core::types::ui::{RectCorner, RefZone, Side};

/// never runs. `ev.prevent_default()` only suppresses the browser default.
pub(super) fn handle_formula_ref_mousedown(
    ev: &web_sys::MouseEvent,
    ref_idx: usize,
    zone: RefZone,
    grab_row: i32,
    grab_col: i32,
    _model: ModelStore,
    state: WorkbookState,
) {
    let Some(editing) = state.editing_cell.get_untracked() else {
        return;
    };
    let Some(active_ref) = editing.formula_analysis.refs().get(ref_idx) else {
        return;
    };
    let anchor = active_ref.sheet_area;
    // Pin grab_cell's sheet to the anchor's sheet — one source of truth for
    // the drag's sheet. The selected sheet can't shift mid-drag while the
    // editor is open, but the invariant lives in the type, not in another
    // invariant.
    let grab_cell = CellAddress {
        sheet: anchor.sheet,
        row: grab_row,
        column: grab_col,
    };
    state.drag.set(DragState::DraggingFormulaRef {
        ref_idx,
        zone,
        anchor,
        grab_cell,
    });
    state.dragged_ref_override.set(Some(RefOverride {
        idx: ref_idx,
        range: anchor,
    }));
    ev.prevent_default();
}

/// Splice the dropped ref's new text into the formula at mouseup.
///
/// Builds a new `RefNode` via `RefNode::with_area` so `$`-flags and
/// `Sheet!` prefix survive, stringifies it, and splices through the same
/// `splice_ref` keystroke buffer the point-mode drag uses. The drop-on-
/// origin no-op (drop coincides with the ref's current area — Excel
/// ignores it) is decided inside `splice_dragged_ref`, which returns
/// `None` and short-circuits the rewrite. Re-runs `analyze_formula` on
/// the new text; the reactive subscription on `editing_cell` then
/// republishes the overlay with refs at their new positions.
pub(super) fn commit_formula_ref_drag(
    ref_idx: usize,
    new_range: SheetRange,
    model: ModelStore,
    state: WorkbookState,
) {
    let Some(edit) = state.editing_cell.get_untracked() else {
        return;
    };
    let Some(active_ref) = edit.formula_analysis.refs().get(ref_idx) else {
        return;
    };
    let original_ref = active_ref.ref_node.clone();
    let span = active_ref.span;
    let Some((new_text, new_span)) =
        splice_dragged_ref(&edit.text, span, &original_ref, new_range, edit.address)
    else {
        // Drop-on-origin no-op (drop coincides with the ref's current area).
        return;
    };
    state.editing_cell.update(|c| {
        if let Some(e) = c {
            e.cursor = new_span.end;
            e.formula_analysis = model.with_value(|m| m.analyze_at(&new_text, e.address));
            e.text = new_text;
        }
    });
}

/// Compute the dragged ref's new range from the grab zone and the cursor cell.
///
/// `zone` picks which part of `anchor` moves (a corner/edge for a resize, the
/// whole body for a translate); the result is clamped at the sheet origin
/// rather than producing zero-based addresses.
pub(crate) fn dragged_ref_range(
    anchor: SheetRange,
    zone: RefZone,
    grab_cell: CellAddress,
    cursor: CellAddress,
) -> SheetRange {
    let a = anchor.area;
    let cell = |r: i32, c: i32, r2: i32, c2: i32| {
        let r1 = r.max(1).min(r2);
        let c1 = c.max(1).min(c2);
        let r2 = r2.clamp(1, LAST_ROW);
        let c2 = c2.clamp(1, LAST_COLUMN);
        SheetRange::new(anchor.sheet, r1, c1, r2, c2)
    };
    match zone {
        RefZone::Body => {
            // Drag math operates on the visually painted rect — normalize so
            // a user-typed `B6:B4` translates the same as `B4:B6`.
            let n = a.normalized();
            let dr = cursor.row - grab_cell.row;
            let dc = cursor.column - grab_cell.column;
            // Clamp the leading corner; apply the *clamped* delta to the
            // trailing corner so width/height stay constant (Excel-like move).
            let max_r1 = LAST_ROW - n.height() + 1;
            let max_c1 = LAST_COLUMN - n.width() + 1;
            let new_r1 = (n.r1 + dr).clamp(1, max_r1);
            let new_c1 = (n.c1 + dc).clamp(1, max_c1);
            let actual_dr = new_r1 - n.r1;
            let actual_dc = new_c1 - n.c1;
            let new_r2 = n.r2 + actual_dr;
            let new_c2 = n.c2 + actual_dc;
            SheetRange::new(anchor.sheet, new_r1, new_c1, new_r2, new_c2)
        }
        RefZone::Edge(Side::Top) => cell(cursor.row, a.c1, a.r2, a.c2),
        RefZone::Edge(Side::Bottom) => cell(a.r1, a.c1, cursor.row, a.c2),
        RefZone::Edge(Side::Left) => cell(a.r1, cursor.column, a.r2, a.c2),
        RefZone::Edge(Side::Right) => cell(a.r1, a.c1, a.r2, cursor.column),
        RefZone::Corner(RectCorner::TopLeft) => cell(cursor.row, cursor.column, a.r2, a.c2),
        RefZone::Corner(RectCorner::TopRight) => cell(cursor.row, a.c1, a.r2, cursor.column),
        RefZone::Corner(RectCorner::BottomLeft) => cell(a.r1, cursor.column, cursor.row, a.c2),
        RefZone::Corner(RectCorner::BottomRight) => cell(a.r1, a.c1, cursor.row, cursor.column),
    }
}
