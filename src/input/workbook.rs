//! Workbook-level lifecycle actions: switch, create, delete.
//!
//! These operations replace the loaded model entirely — unlike spreadsheet
//! actions that mutate cells within it. Each also resets transient UI state
//! (editing buffer, drag) so stale state doesn't bleed across workbooks.

use ironcalc_base::UserModel;
use leptos::prelude::{UpdateValue, WithValue};

use crate::app_state::AppState;
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::state::{DragState, ModelStore, WorkbookState};
use crate::storage::{self, WorkbookId};

/// Workbook-level lifecycle operations.
///
/// Separate from `SpreadsheetAction` because these replace the loaded model
/// rather than mutate cells within it. Callers are responsible for any
/// confirmation dialog before calling `Delete`.
pub enum WorkbookAction {
    /// Save the current workbook and load a different one.
    Switch(WorkbookId),
    /// Save the current workbook and activate a fresh blank one.
    Create,
    /// Delete a workbook. If it's active, load or create a replacement.
    Delete(WorkbookId),
}

/// Execute a [`WorkbookAction`], replacing the loaded model and resetting
/// transient UI state.
pub fn execute_workbook(
    action: &WorkbookAction,
    model: ModelStore,
    state: &WorkbookState,
    app: AppState,
) {
    match action {
        WorkbookAction::Switch(target_uuid) => {
            let cur = state.current_uuid.get_untracked();
            if cur == Some(*target_uuid) {
                return;
            }
            if let Some(uuid) = &cur {
                model.with_value(|m| storage::save(uuid, m));
            }
            if let Some(new_model) = storage::load(target_uuid) {
                activate(*target_uuid, new_model, model, state);
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
            let is_current = state.current_uuid.get_untracked() == Some(*uuid);
            storage::delete(uuid);
            if is_current {
                let (next_uuid, next_model) =
                    storage::load_selected().unwrap_or_else(storage::create_new);
                activate(next_uuid, next_model, model, state);
            }
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
    state.editing_cell.set(None);
    state.drag.set(DragState::Idle);
    state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
}
