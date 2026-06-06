//! Edit actions: start/commit/cancel cell editing.

use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::coord::{CellAddress, SheetRange};
use crate::events::{ContentEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::EditError;
use crate::input::formula::FormulaAnalysis;
use crate::model::{
    ArrowKey, EvaluationMode, FormulaAnalyzer, Navigator, SheetQuery, mutate,
    style_types::{BooleanValue, StylePath},
    try_mutate,
};
use crate::state::{DragState, EditingCell, ModelStore, WorkbookState};
use crate::state::{EditFocus, EditMode};

/// Cell edit lifecycle actions.
#[derive(Debug, Clone, PartialEq)]
pub enum EditAction {
    Start(String),
    EnterEditMode,
    /// Enter/Tab: write the edit buffer to the model then navigate.
    CommitAndNavigate(ArrowKey),
    /// Ctrl+Shift+Enter: commit the edit buffer as a CSE array formula sized
    /// to the current selection, then navigate. Single-cell selection = 1x1.
    CommitArrayAndNavigate(ArrowKey),
    /// Escape: discard the edit buffer without writing to the model.
    Cancel,
}

/// Dispatch an [`EditAction`] against the model and UI state.
///
/// Emits typed events after successful transitions. Returns `Err(EditError)`
/// when `set_user_input` fails on commit.
pub fn execute_edit(
    action: &EditAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), EditError> {
    match action {
        EditAction::Start(text) => {
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::EditingStarted {
                    address: model.with_value(|m| {
                        let address = m.active_cell();
                        state.editing_cell.set(Some(EditingCell {
                            address,
                            cursor: text.len(),
                            text: text.clone(),
                            mode: EditMode::Accept,
                            focus: EditFocus::Cell,
                            text_dirty: true,
                            formula_analysis: FormulaAnalysis::default(),
                        }));
                        address
                    }),
                },
            ));
        }
        EditAction::EnterEditMode => {
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::EditingStarted {
                    address: model.with_value(|m| {
                        let v = m.get_selected_view();
                        let text = m
                            .get_cell_content(v.sheet, v.row, v.column)
                            .unwrap_or_default();
                        let address = m.active_cell();

                        // Then set formula_analysis: analysis on the EditingCell being constructed/updated

                        state.editing_cell.set(Some(EditingCell {
                            address,
                            cursor: text.len(),
                            text: text.clone(),
                            mode: EditMode::Edit,
                            focus: EditFocus::Cell,
                            text_dirty: false,
                            formula_analysis: model.with_value(|m| m.analyze_in_context(&text)),
                        }));
                        address
                    }),
                },
            ));
        }
        EditAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                stamp_last_formula(&edit.text);
                try_mutate(
                    model,
                    EvaluationMode::Immediate,
                    |m| -> Result<(), EditError> {
                        m.set_user_input(
                            edit.address.sheet,
                            edit.address.row,
                            edit.address.column,
                            &edit.text,
                        )
                        .map_err(EditError::Engine)?;
                        // Excel parity: an Alt+Enter value carries an embedded
                        // newline, so force wrap-text on the cell — otherwise the
                        // extra lines render but stay clipped to the row height.
                        if edit.text.contains('\n') {
                            let area = edit.address.to_sheet_area().to_ironcalc_area();
                            m.update_range_style(
                                &area,
                                StylePath::WRAP_TEXT.as_str(),
                                BooleanValue::True.as_str(),
                            )
                            .map_err(EditError::Engine)?;
                        }
                        Ok(())
                    },
                )?;
                finish_commit(model, state, &edit, *dir);
            }
        }
        EditAction::CommitArrayAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                stamp_last_formula(&edit.text);
                try_mutate(model, EvaluationMode::Immediate, |m| {
                    commit_array_formula(m, &edit)
                })?;
                finish_commit(model, state, &edit, *dir);
            }
        }
        EditAction::Cancel => {
            // Escape during a formula-ref drag aborts the drag only —
            // formula text is untouched (the splice hasn't run), editing
            // stays alive, the user can keep typing. Without this branch,
            // Escape would unconditionally close the edit and lose the
            // in-progress formula.
            if matches!(
                state.drag.get_untracked(),
                DragState::DraggingFormulaRef { .. }
            ) {
                state.drag.set(DragState::Idle);
                state.dragged_ref_override.set(None);
                return Ok(());
            }

            let edit_address = state.editing_cell.get_untracked().map(|e| e.address);
            state.editing_cell.set(None);
            state.drag.set(DragState::Idle);

            if let Some(address) = edit_address {
                state.emit_event(SpreadsheetEvent::Navigation(
                    NavigationEvent::EditingEnded {
                        address,
                        committed: false,
                    },
                ));
            }

            crate::util::refocus_workbook();
        }
    }
    Ok(())
}

/// Commit `edit.text` as a CSE (Ctrl+Shift+Enter) array formula.
///
/// Runs inside `try_mutate`'s closure, so `m` is the live model. The array must
/// be sized to the user's current selection — a single-cell selection produces
/// a 1x1 array, a multi-cell selection fills the whole rectangle.
fn commit_array_formula(m: &mut UserModel<'static>, edit: &EditingCell) -> Result<(), EditError> {
    // Anchor the CSE array at the selection's top-left and fill the whole
    // rectangle — Excel's behavior. The selection (not `edit.address`) is the
    // source of truth: the user frames the array by selecting the range first,
    // and IronCalc rejects writes into a cell that's already part of an array,
    // so the corner is the only valid anchor. A single-cell selection yields a
    // 1x1 array.
    let SheetRange { sheet, area } = SheetRange::from_view(m);
    let area = area.normalized();
    m.set_user_array_formula(
        sheet,
        area.r1,
        area.c1,
        area.width(),
        area.height(),
        &edit.text,
    )
    .map_err(EditError::Engine)
}

/// Shared post-write commit steps: clear edit state, navigate one cell in `dir`,
/// emit the content/mode/navigation events as one batch, and refocus the grid.
/// Extracted so the value-commit and array-commit arms can't drift apart.
fn finish_commit(model: ModelStore, state: &WorkbookState, edit: &EditingCell, dir: ArrowKey) {
    state.editing_cell.set(None);
    state.drag.set(DragState::Idle);

    mutate(model, EvaluationMode::Deferred, |m| m.nav_arrow(dir));

    let nav_address = model.with_value(CellAddress::from_view);
    state.emit_events(vec![
        SpreadsheetEvent::Content(ContentEvent::CellChanged {
            // The edited cell — `nav_arrow` above already moved the active
            // cell, so `active_cell()` here would report where the cursor
            // landed, not what changed. Consumers (row-height autofit) key
            // off the edited row.
            address: edit.address,
            old_value: None,
            new_value: Some(edit.text.clone()),
        }),
        SpreadsheetEvent::Navigation(NavigationEvent::EditingEnded {
            address: model.with_value(|m| m.active_cell()),
            committed: true,
        }),
        SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged {
            address: nav_address,
        }),
    ]);

    crate::util::refocus_workbook();
}

/// Stamp the last-committed text for the dev-tools PerfPanel readout. Phase
/// timestamps (commit_start / input_done / eval_done) are written inside
/// `try_mutate` itself. No-op without the `dev-tools` feature.
#[cfg(feature = "dev-tools")]
fn stamp_last_formula(text: &str) {
    if let Some(perf) = leptos::prelude::use_context::<crate::perf::PerfTimings>() {
        leptos::prelude::Set::set(&perf.last_formula, Some(text.to_owned()));
    }
}

#[cfg(not(feature = "dev-tools"))]
fn stamp_last_formula(_text: &str) {}
