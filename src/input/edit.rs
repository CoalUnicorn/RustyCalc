//! Edit actions: start/commit/cancel cell editing.

use leptos::prelude::*;

use crate::coord::CellAddress;
use crate::events::{ContentEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::EditError;
use crate::model::{mutate, try_mutate, ArrowKey, EvaluationMode, FrontendModel};
use crate::state::{DragState, EditingCell, ModelStore, WorkbookState};
use crate::state::{EditFocus, EditMode};

/// Cell edit lifecycle actions.
#[derive(Debug, Clone, PartialEq)]
pub enum EditAction {
    Start(String),
    EnterEditMode,
    /// Enter/Tab: write the edit buffer to the model then navigate.
    CommitAndNavigate(ArrowKey),
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
                            text: text.clone(),
                            mode: EditMode::Accept,
                            focus: EditFocus::Cell,
                            text_dirty: true,
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
                        state.editing_cell.set(Some(EditingCell {
                            address,
                            text,
                            mode: EditMode::Edit,
                            focus: EditFocus::Cell,
                            text_dirty: false,
                        }));
                        address
                    }),
                },
            ));
        }
        EditAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                // Write the edit buffer to the model and recalculate (timed).
                // let perf = expect_context::<AppState>().perf;
                // perf.last_formula.set(Some(edit.text.clone()));
                // Write the edit buffer to the model and recalculate.
                try_mutate(model, EvaluationMode::Immediate, |m| {
                    m.set_user_input(
                        edit.address.sheet,
                        edit.address.row,
                        edit.address.column,
                        &edit.text,
                    )
                    .map_err(EditError::Engine)
                })?;

                // Clear all edit-related state.
                state.editing_cell.set(None);
                state.drag.set(DragState::Idle);

                // Navigate to the next cell.
                mutate(model, EvaluationMode::Deferred, |m| m.nav_arrow(*dir));

                // Fire content + mode + navigation together so EventBus signals update once.
                let nav_address = model.with_value(CellAddress::from_view);
                state.emit_events(vec![
                    SpreadsheetEvent::Content(ContentEvent::CellChanged {
                        address: model.with_value(|m| m.active_cell()),
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
        }
        EditAction::Cancel => {
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
