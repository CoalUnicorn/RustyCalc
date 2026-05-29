//! Hit-test-resolved click helpers.
//!
//! Called from `mousedown` once `IronCanvas::hit_test` classifies the
//! click. These four are also exposed at the module facade because the
//! header context menu (right-click → "Select column") synthesises
//! header-click events without going through `mousedown`.

use leptos::prelude::*;

use crate::coord::{CellAddress, RefNode, SheetRange, TextRef};
use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::input::formula::{is_in_reference_mode, splice_ref};
use crate::model::{FormulaAnalyzer, Navigator};
use crate::state::{
    DragState, EditMode, ModelStore, WorkbookState,
};

/// Click on the top-left corner cell: select the entire sheet.
pub fn handle_corner_click(model: ModelStore, state: WorkbookState) {
    web_sys::console::time_with_label("corner:nav_select_all");
    model.update_value(|m| {
        m.nav_select_all();
    });
    web_sys::console::time_end_with_label("corner:nav_select_all");

    web_sys::console::time_with_label("corner:editing_cell");

    state.editing_cell.set(None);
    web_sys::console::time_end_with_label("corner:editing_cell");

    web_sys::console::time_with_label("corner:from_view");
    let sheet_area = model.with_value(SheetRange::from_view);
    web_sys::console::time_end_with_label("corner:from_view");

    web_sys::console::time_with_label("corner:emit_event");
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
    web_sys::console::time_end_with_label("corner:emit_event");
}

/// Click on a column header: select the entire column, or extend the current
/// selection if Shift is held. `col` is the column index resolved by the
/// dispatcher's `IronCanvas::hit_test` against the painted frame.
pub fn handle_col_header_click(
    ev: &web_sys::MouseEvent,
    col: i32,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        if ev.shift_key() {
            m.nav_extend_column_selection(col);
        } else {
            m.nav_select_column(col);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetRange::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click on a row header: select the entire row, or extend the current
/// selection if Shift is held.
pub fn handle_row_header_click(
    ev: &web_sys::MouseEvent,
    row: i32,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        if ev.shift_key() {
            m.nav_extend_row_selection(row);
        } else {
            m.nav_select_row(row);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetRange::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click in the cell area: handles point-mode formula entry, autofill handle
/// drag start, Shift-click range extension, and regular single-cell navigation.
///
/// `row` / `col` are the cell under the cursor, resolved upstream by
/// `IronCanvas::hit_test` against the painted frame. `near_handle` is `true`
/// iff the dispatcher classified the hit as `HitTest::AutofillHandle`.
pub fn handle_cell_click(
    ev: &web_sys::MouseEvent,
    row: i32,
    col: i32,
    near_handle: bool,
    model: ModelStore,
    state: WorkbookState,
) {
    // Point mode: intercept click during formula entry.
    // When the cursor is at a syntactically valid reference position inside
    // a formula, clicking a cell inserts/replaces the reference rather than
    // committing the edit and navigating away.
    if let Some(ref edit) = state.editing_cell.get_untracked() {
        let already_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });
        let may_point = edit.mode == EditMode::Accept || edit.text_dirty || already_pointing;
        if may_point {
            let cursor = edit.cursor;
            // Caret-hit: if the cursor sits on an existing resolved ref,
            // the click REPLACES that ref in place — preserving its `$`
            // flags and sheet qualification via `relocate_to`.
            let caret_hit = if !already_pointing {
                edit.formula_analysis.refs_at_cursor(cursor).next().cloned()
            } else {
                None
            };
            if already_pointing || caret_hit.is_some() || is_in_reference_mode(&edit.text, cursor) {
                let editing = model.with_value(CellAddress::from_view);
                let (ref_node, prev_span) = if let Some(hit) = caret_hit {
                    (hit.ref_node.relocate_to(row, col, &editing), Some(hit.span))
                } else if let DragState::Pointing { ref_text, .. } = state.drag.get_untracked() {
                    (
                        RefNode::from_cell_area(
                            SheetRange::from_cell(editing.sheet, row, col),
                            editing,
                            "",
                        ),
                        Some(ref_text),
                    )
                } else {
                    (
                        RefNode::from_cell_area(
                            SheetRange::from_cell(editing.sheet, row, col),
                            editing,
                            "",
                        ),
                        None,
                    )
                };
                let ref_str = ref_node.to_localized(&editing.as_stringify_ctx());
                let text = edit.text.clone();
                let (new_text, ref_text) =
                    splice_ref(&text, prev_span.unwrap_or(TextRef::at(cursor)), &ref_str);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.cursor = ref_text.end;
                        e.text = new_text.clone();
                        e.formula_analysis = model.with_value(|m| m.analyze_in_context(&new_text));
                    }
                });
                state.drag.set(DragState::Pointing { ref_node, ref_text });
                return;
            }
        }
    }

    if near_handle {
        // Begin autofill drag - don't change the selection.
        state.drag.set(DragState::Extending {
            to_row: row,
            to_col: col,
        });
    } else if ev.shift_key() {
        // Shift-click extends the range from the current anchor.
        model.update_value(|m| {
            m.nav_extend_selection(row, col);
        });
        state.drag.set(DragState::Selecting);
    } else {
        model.update_value(|m| {
            m.nav_set_cell(row, col);
        });
        state.drag.set(DragState::Selecting);
    }

    state.editing_cell.set(None);

    // Emit the appropriate navigation event so toolbar/formula-bar
    // update and the canvas repaints via visual_events.
    // Autofill start: drag state change alone triggers the canvas repaint; no navigation event.
    if !near_handle {
        if ev.shift_key() {
            let sheet_area = model.with_value(SheetRange::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        } else {
            let address = model.with_value(CellAddress::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionChanged { address },
            ));
        }
    }
}
