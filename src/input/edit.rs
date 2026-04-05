//! Edit actions: start/commit/cancel cell editing.

use leptos::prelude::{WithValue, *};

use crate::events::{ContentEvent, DragState, ModeEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::EditError;
use crate::input::helpers::{mutate, EvaluationMode};
use crate::model::{ArrowKey, CellAddress, FrontendModel};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::storage;

#[derive(Debug, Clone, PartialEq)]
pub enum EditAction {
    /// Printable key: start a new edit with this character as the initial text.
    Start(String),
    /// F2: enter edit mode preserving the existing cell content.
    EnterEditMode,
    /// Enter/Tab: write the edit buffer to the model then navigate.
    CommitAndNavigate(ArrowKey),
    /// Escape: discard the edit buffer without writing to the model.
    Cancel,
}

pub fn execute_edit(
    action: &EditAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), EditError> {
    match action {
        EditAction::Start(text) => {
            let address = model.with_value(|m| {
                let address = CellAddress::from_view(m);

                state.editing_cell.set(Some(EditingCell {
                    address,
                    text: text.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::Cell,
                }));
                address
            });

            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditStarted { address }));
        }
        EditAction::EnterEditMode => {
            let address = model.with_value(|m| {
                let v = m.get_selected_view();
                let text = m
                    .get_cell_content(v.sheet, v.row, v.column)
                    .unwrap_or_default();

                let address = CellAddress::from_view(m);

                state.editing_cell.set(Some(EditingCell {
                    address,
                    text,
                    mode: EditMode::Edit,
                    focus: EditFocus::Cell,
                }));
                address
            });

            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditStarted { address }));
        }
        EditAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                // Perf timing
                let perf = state.perf;
                perf.commit_start.set(Some(crate::perf::now()));
                perf.last_formula.set(Some(edit.text.clone()));

                // Write the edit buffer to the model and recalculate.
                // pause_evaluation() prevents set_user_input from triggering an internal
                // evaluate() call — without it we'd evaluate twice (once inside
                // set_user_input, once explicitly below).
                let mut commit_result: Result<(), EditError> = Ok(());
                model.update_value(|m| {
                    m.pause_evaluation();
                    commit_result = m
                        .set_user_input(
                            edit.address.sheet,
                            edit.address.row,
                            edit.address.column,
                            &edit.text,
                        )
                        .map_err(EditError::Engine);
                    perf.input_done.set(Some(crate::perf::now()));
                    m.resume_evaluation();
                    m.evaluate();
                    perf.eval_done.set(Some(crate::perf::now()));
                });
                // Propagate any engine error before touching reactive state.
                commit_result?;

                // Clear all edit-related state.
                state.editing_cell.set(None);
                state.drag.set(DragState::Idle);

                // Persist the committed change immediately.
                if let Some(uuid) = state.current_uuid.get_untracked() {
                    model.with_value(|m| storage::save(&uuid, m));
                }

                // Navigate to the next cell.
                mutate(model, state, EvaluationMode::Deferred, |m| {
                    m.nav_arrow(*dir)
                });

                // Fire content + mode + navigation together so EventBus signals update once.
                let nav_address = model
                    .with_value(|m: &ironcalc_base::UserModel<'static>| CellAddress::from_view(m));
                state.emit_events(vec![
                    SpreadsheetEvent::Content(ContentEvent::CellChanged {
                        address: CellAddress::from_editing(&edit),
                        old_value: None,
                        new_value: Some(edit.text.clone()),
                    }),
                    SpreadsheetEvent::Mode(ModeEvent::EditEnded),
                    SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged {
                        address: nav_address,
                    }),
                ]);

                crate::util::refocus_workbook();
            }
        }
        EditAction::Cancel => {
            state.editing_cell.set(None);
            state.drag.set(DragState::Idle);

            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditEnded));

            crate::util::refocus_workbook();
        }
    }
    Ok(())
}
