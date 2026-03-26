//! Edit actions: start/commit/cancel cell editing.

use crate::input::helpers::{mutate, Eval};
use crate::model::{ArrowKey, FrontendModel};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::storage;
use crate::util::warn_if_err;

use leptos::prelude::*;

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
            model.with_value(|m| {
                let v = m.get_selected_view();
                state.editing_cell.set(Some(EditingCell {
                    sheet: v.sheet,
                    row: v.row,
                    col: v.column,
                    text: text.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::Cell,
                }));
            });
            state.request_redraw();
        }
        EditAction::EnterEditMode => {
            model.with_value(|m| {
                let v = m.get_selected_view();
                let text = m
                    .get_cell_content(v.sheet, v.row, v.column)
                    .unwrap_or_default();
                state.editing_cell.set(Some(EditingCell {
                    sheet: v.sheet,
                    row: v.row,
                    col: v.column,
                    text,
                    mode: EditMode::Edit,
                    focus: EditFocus::Cell,
                }));
            });
            state.request_redraw();
        }
        EditAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                // ── Perf timing ──────────────────────────────────────────
                let perf = state.perf;
                perf.commit_start.set(Some(crate::perf::now()));
                perf.last_formula.set(Some(edit.text.clone()));

                // Write the edit buffer to the model and recalculate.
                model.update_value(|m| {
                    warn_if_err(
                        m.set_user_input(edit.sheet, edit.row, edit.col, &edit.text),
                        "set_user_input",
                    );
                    perf.input_done.set(Some(crate::perf::now()));
                    m.evaluate();
                    perf.eval_done.set(Some(crate::perf::now()));
                });

                // Clear all edit-related state.
                state.editing_cell.set(None);
                state.point_range.set(None);
                state.point_ref_span.set(None);
                
                // Persist the committed change immediately.
                if let Some(uuid) = state.current_uuid.get_untracked() {
                    model.with_value(|m| storage::save(&uuid, m));
                }
                
                // Navigate to the next cell and redraw.
                mutate(model, state, Eval::No, |m| m.nav_arrow(*dir));
                crate::util::refocus_workbook();
            }
        }
        EditAction::Cancel => {
            state.editing_cell.set(None);
            state.point_range.set(None);
            state.point_ref_span.set(None);
            state.request_redraw();
            crate::util::refocus_workbook();
        }
    }
}
