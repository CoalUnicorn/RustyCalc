//! Workbook-level lifecycle actions: switch, create, delete.
//!
//! These operations replace the loaded model entirely — unlike spreadsheet
//! actions that mutate cells within it. Each also resets transient UI state
//! (editing buffer, drag) so stale state doesn't bleed across workbooks.

use ironcalc_base::UserModel;
use leptos::prelude::{UpdateValue, WithValue};

use crate::app_state::AppState;
use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
use crate::storage::{self, WorkbookId};

/// Workbook-level lifecycle operations.
///
/// Separate from `SpreadsheetAction` because these replace the loaded model
/// rather than mutate cells within it. Callers are responsible for any
/// confirmation dialog before calling `Delete`.
#[allow(clippy::large_enum_variant)]
pub enum WorkbookAction {
    /// Save the current workbook and load a different one.
    Switch(WorkbookId),
    /// Save the current workbook and activate a fresh blank one.
    Create,
    /// Delete a workbook. If it's active, load or create a replacement.
    Delete(WorkbookId),
    /// Save the current workbook and activate `model` under a fresh UUID.
    Import(UserModel<'static>),
}

/// Execute a [`WorkbookAction`], replacing the loaded model and resetting
/// transient UI state.
pub fn execute_workbook(
    action: WorkbookAction,
    model: ModelStore,
    state: &WorkbookState,
    app: AppState,
) {
    match action {
        WorkbookAction::Switch(target_uuid) => {
            let cur = state.current_uuid.get_untracked();
            if cur == Some(target_uuid) {
                return;
            }
            // Save the current workbook before switching. If encoding fails
            // (e.g. model exceeds MAX_STORAGE_BYTES), warn the user instead of
            // silently losing changes.
            if let Some(uuid) = &cur {
                let saved = model.with_value(|m| {
                    let bytes = m.to_bytes();
                    if bytes.len() > storage::MAX_STORED_BYTES {
                        state.status.set(Some(StatusMessage::Error(format!(
                            "Cannot save workbook — it exceeds the {} MB limit. \
                             Delete some sheets or data before switching.",
                            storage::MAX_STORED_BYTES / 1_000_000
                        ))));
                        return false;
                    }
                    storage::save(uuid, m);
                    true
                });
                if !saved {
                    return;
                }
            }
            match storage::load(&target_uuid) {
                Some(new_model) => activate(target_uuid, new_model, model, state),
                None => {
                    state.status.set(Some(StatusMessage::Error(
                        "Cannot open workbook — it was saved with an older version. \
                         Your data is still in browser storage but needs migration."
                            .to_string(),
                    )));
                }
            }
        }
        WorkbookAction::Create => {
            if let Some(uuid) = state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
            let (new_uuid, new_model) = storage::create_new();
            activate(new_uuid, new_model, model, state);
            app.bump_registry();
        }
        WorkbookAction::Delete(uuid) => {
            let is_current = state.current_uuid.get_untracked() == Some(uuid);
            storage::delete(&uuid);
            if is_current {
                match storage::load_selected() {
                    Some((next_uuid, next_model)) => {
                        activate(next_uuid, next_model, model, state);
                    }
                    None => {
                        let (next_uuid, next_model) = storage::create_new();
                        activate(next_uuid, next_model, model, state);
                        state.status.set(Some(StatusMessage::Error(
                            "Other workbooks could not be loaded after deletion. \
                             Created a new blank workbook."
                                .to_string(),
                        )));
                    }
                }
            }
            app.bump_registry();
        }
        WorkbookAction::Import(new_model) => {
            if let Some(uuid) = state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
            let (new_uuid, new_model) =
                storage::create_new_from(new_model, storage::WorkbookOrigin::FileImport);
            activate(new_uuid, new_model, model, state);
            app.bump_registry();
        }
    }
}

// Common model-replacement sequence shared by all three operations:
// load the new model, mark it selected in localStorage, update all reactive
// state, and request a canvas repaint.
fn activate(
    uuid: WorkbookId,
    new_model: UserModel<'static>,
    model: ModelStore,
    state: &WorkbookState,
) {
    model.update_value(|m| *m = new_model);
    storage::set_selected_uuid(&uuid);
    state.current_uuid.set(Some(uuid));
    state.reset_view_state();
    state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));
}
