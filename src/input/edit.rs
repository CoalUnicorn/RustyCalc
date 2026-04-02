//! Edit actions: start/commit/cancel cell editing.

use leptos::prelude::{WithValue, *};

use crate::events::{ContentEvent, ModeEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::helpers::{mutate, Eval};
use crate::model::{ArrowKey, CellAddress, FrontendModel};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::storage;
use crate::util::warn_if_err;

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

pub fn execute_edit(action: &EditAction, model: ModelStore, state: &WorkbookState) {
    match action {
        EditAction::Start(text) => {
            let address = model.with_value(|m| {
                let address = CellAddress::from_view(m);

                state.set_editing_cell(Some(EditingCell {
                    address,
                    text: text.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::Cell,
                }));
                address
            });

            // Fire mode event for edit start
            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditStarted { address }));
        }
        EditAction::EnterEditMode => {
            let address = model.with_value(|m| {
                let v = m.get_selected_view();
                let text = m
                    .get_cell_content(v.sheet, v.row, v.column)
                    .unwrap_or_default();

                let address = CellAddress::from_view(m);

                state.set_editing_cell(Some(EditingCell {
                    address: address,
                    text,
                    mode: EditMode::Edit,
                    focus: EditFocus::Cell,
                }));
                address
            });

            // Fire mode event for edit mode entry
            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditStarted { address }));
        }
        EditAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.get_editing_cell_untracked() {
                // Perf timing
                let perf = state.perf;
                perf.commit_start.set(Some(crate::perf::now()));
                perf.last_formula.set(Some(edit.text.clone()));

                // Write the edit buffer to the model and recalculate.
                model.update_value(|m| {
                    warn_if_err(
                        m.set_user_input(
                            edit.address.sheet,
                            edit.address.row,
                            edit.address.column,
                            &edit.text,
                        ),
                        "set_user_input",
                    );
                    perf.input_done.set(Some(crate::perf::now()));
                    m.evaluate();
                    perf.eval_done.set(Some(crate::perf::now()));
                });

                // Fire content changed event for cell edit commit
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::CellChanged {
                    address: CellAddress::from_editing(&edit),
                    old_value: None,
                    new_value: Some(edit.text.clone()),
                }));

                // Clear all edit-related state.
                state.set_editing_cell(None);
                state.set_point_range(None);
                state.set_point_ref_span(None);

                // Fire mode event for edit end
                state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditEnded));

                // Persist the committed change immediately.
                if let Some(uuid) = state.get_current_uuid_untracked() {
                    model.with_value(|m| storage::save(&uuid, m));
                }

                // Navigate to the next cell.
                mutate(model, state, Eval::No, |m| m.nav_arrow(*dir));

                // Fire navigation event for post-edit navigation
                let address = model
                    .with_value(|m: &ironcalc_base::UserModel<'static>| CellAddress::from_view(m));

                state.emit_event(SpreadsheetEvent::Navigation(
                    NavigationEvent::SelectionChanged { address },
                ));

                crate::util::refocus_workbook();
            }
        }
        EditAction::Cancel => {
            state.set_editing_cell(None);
            state.set_point_range(None);
            state.set_point_ref_span(None);

            // Fire mode event for edit cancellation
            state.emit_event(SpreadsheetEvent::Mode(ModeEvent::EditEnded));

            crate::util::refocus_workbook();
        }
    }
}
