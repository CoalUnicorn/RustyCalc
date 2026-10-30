//! Workbook lifecycle operations await durable storage before replacing the model.

use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::app_state::AppState;
use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
use crate::storage::{self, WorkbookGroup, WorkbookId};

#[allow(clippy::large_enum_variant)]
pub enum WorkbookAction {
    Switch(WorkbookId),
    Create,
    Delete(WorkbookId),
    Import(UserModel<'static>),
    Share(UserModel<'static>),
    Rename(WorkbookId, String),
    Group(WorkbookId, WorkbookGroup),
    Trust(WorkbookId),
}

/// Ignore duplicate actions while storage is busy. The application also blocks input.
pub fn execute_workbook(
    action: WorkbookAction,
    model: ModelStore,
    state: &WorkbookState,
    app: AppState,
) {
    if app.storage_busy.get_untracked() {
        return;
    }
    app.storage_busy.set(true);
    let state = *state;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = run_workbook(action, model, state, app).await {
            state.status.set(Some(StatusMessage::Error(error)));
        }
        app.storage_busy.set(false);
    });
}

async fn save_current(model: ModelStore, state: WorkbookState) -> Result<(), String> {
    if let Some(uuid) = state.current_uuid.get_untracked() {
        model
            .with_value(|m| storage::snapshot(uuid, m))?
            .save()
            .await?;
    }
    Ok(())
}

/// The caller holds `storage_busy` until completion, including share-modal callers.
pub async fn run_workbook(
    action: WorkbookAction,
    model: ModelStore,
    state: WorkbookState,
    app: AppState,
) -> Result<(), String> {
    let origin = if matches!(&action, WorkbookAction::Share(_)) {
        storage::WorkbookOrigin::ShareLink
    } else {
        storage::WorkbookOrigin::FileImport
    };
    match action {
        WorkbookAction::Switch(target) => {
            if state.current_uuid.get_untracked() == Some(target) {
                return Ok(());
            }
            save_current(model, state).await?;
            let loaded = storage::load(&target).await?;
            activate(target, loaded, model, &state);
        }
        WorkbookAction::Create => {
            save_current(model, state).await?;
            let (uuid, loaded) = storage::create_new().await?;
            activate(uuid, loaded, model, &state);
        }
        WorkbookAction::Delete(uuid) => {
            let current = state.current_uuid.get_untracked() == Some(uuid);
            // Prepare a readable replacement before deleting the active record.
            let replacement = if current {
                let next = storage::load_registry()
                    .keys()
                    .filter(|id| **id != uuid)
                    .min()
                    .copied();
                match next {
                    Some(next) => Some((next, storage::load(&next).await?)),
                    None => None,
                }
            } else {
                None
            };
            if current && replacement.is_none() {
                // If creation fails, the current workbook still exists.
                let previous = storage::get_selected_uuid();
                let (next, loaded) = storage::create_new().await?;
                if let Err(error) = storage::delete(&uuid).await {
                    if let Some(previous) = previous {
                        storage::set_selected_uuid(&previous);
                    }
                    app.bump_registry();
                    return Err(error);
                }
                activate(next, loaded, model, &state);
            } else {
                storage::delete(&uuid).await?;
                if let Some((next, loaded)) = replacement {
                    activate(next, loaded, model, &state);
                }
            }
        }
        WorkbookAction::Import(loaded) | WorkbookAction::Share(loaded) => {
            save_current(model, state).await?;
            let (uuid, loaded) = storage::create_new_from(loaded, origin).await?;
            activate(uuid, loaded, model, &state);
        }
        WorkbookAction::Rename(uuid, name) => {
            let name = storage::sanitize_name(&name);
            // Save current edits first. Rename a detached model so a failed write cannot change the editor.
            save_current(model, state).await?;
            let mut renamed = storage::load(&uuid).await?;
            renamed.set_name(&name);
            storage::snapshot(uuid, &renamed)?.rename().await?;
            if state.current_uuid.get_untracked() == Some(uuid) {
                model.update_value(|m| m.set_name(&name));
            }
        }
        WorkbookAction::Group(uuid, group) => storage::update_group(&uuid, group).await?,
        WorkbookAction::Trust(uuid) => storage::promote_from_shared(&uuid).await?,
    }
    app.bump_registry();
    Ok(())
}

fn activate(
    uuid: WorkbookId,
    new_model: UserModel<'static>,
    model: ModelStore,
    state: &WorkbookState,
) {
    model.update_value(|m| *m = new_model);
    state.advance_workbook_generation();
    storage::set_selected_uuid(&uuid);
    state.current_uuid.set(Some(uuid));
    state.reset_view_state();
    state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));
}
