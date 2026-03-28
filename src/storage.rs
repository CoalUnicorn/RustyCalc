use base64::{engine::general_purpose::STANDARD, Engine};
use gloo_storage::{LocalStorage, Storage};
use ironcalc_base::UserModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Log a storage error to the browser console and discard the `Err`.
/// Used in place of bare `.ok()` so silent failures become visible in DevTools.
fn log_err<E: std::fmt::Display>(result: Result<(), E>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[ironcalc storage] {ctx}: {e}").into());
    }
}

// localStorage key constants mirroring storage.ts
const SELECTED_KEY: &str = "selected";
const MODELS_KEY: &str = "models";

/// Per-workbook metadata stored in the "models" registry.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkbookMeta {
    pub name: String,
}

// Registry helpers

/// Load the UUID->metadata registry from localStorage.
pub fn load_registry() -> HashMap<String, WorkbookMeta> {
    LocalStorage::get(MODELS_KEY).unwrap_or_default()
}

fn save_registry(registry: &HashMap<String, WorkbookMeta>) {
    log_err(LocalStorage::set(MODELS_KEY, registry), "save registry");
}

// Selection helpers

/// Return the UUID of the currently selected workbook, if set.
pub fn get_selected_uuid() -> Option<String> {
    LocalStorage::get(SELECTED_KEY).ok()
}

/// Persist the active workbook UUID so it survives page reloads.
pub fn set_selected_uuid(uuid: &str) {
    log_err(LocalStorage::set(SELECTED_KEY, uuid), "set selected uuid");
}

// Core CRUD

/// Serialize `model` to bytes, base64-encode, and write to localStorage.
/// Also refreshes the workbook's entry in the metadata registry.
pub fn save(uuid: &str, model: &UserModel) {
    let bytes = model.to_bytes();
    let encoded = STANDARD.encode(&bytes);
    log_err(LocalStorage::set(uuid, encoded), "save model bytes");

    let mut registry = load_registry();
    registry.insert(
        uuid.to_string(),
        WorkbookMeta {
            name: model.get_name(),
        },
    );
    save_registry(&registry);
}

/// Decode and deserialize a model from localStorage.
/// Returns `None` if the key is absent or the bytes are corrupt.
/// Logs a console warning for decode/parse failures so silent data loss is visible.
pub fn load(uuid: &str) -> Option<UserModel<'static>> {
    let encoded: String = LocalStorage::get(uuid).ok()?;
    let bytes = match STANDARD.decode(encoded) {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[ironcalc storage] load {uuid}: base64 decode failed: {e}").into(),
            );
            return None;
        }
    };
    // "en" is 'static, so the returned UserModel<'static> lifetime is satisfied.
    match UserModel::from_bytes(&bytes, "en") {
        Ok(m) => Some(m),
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[ironcalc storage] load {uuid}: model parse failed: {e}").into(),
            );
            None
        }
    }
}

/// Load the previously selected workbook, falling back to the first available.
/// Returns `None` only when localStorage is completely empty.
pub fn load_selected() -> Option<(String, UserModel<'static>)> {
    // Try the explicitly selected UUID first.
    if let Some(uuid) = get_selected_uuid() {
        if let Some(model) = load(&uuid) {
            return Some((uuid, model));
        }
    }

    // Fall back to the lexicographically first UUID that yields a valid model.
    // Sorting ensures a stable, repeatable result regardless of HashMap iteration order.
    let registry = load_registry();
    let mut uuids: Vec<String> = registry.keys().cloned().collect();
    uuids.sort();
    for uuid in &uuids {
        if let Some(model) = load(uuid) {
            set_selected_uuid(uuid);
            return Some((uuid.clone(), model));
        }
    }

    None
}

/// Create a fresh blank workbook, persist it, set it as selected.
/// The workbook is named "Workbook N" where N is one more than the current registry size.
pub fn create_new() -> (String, UserModel<'static>) {
    let registry = load_registry();
    // `leak()` gives a `&'static str` so UserModel<'static> can borrow it.
    // FIXME: each call leaks a small heap allocation that is never reclaimed.
    // In a typical session users create at most a handful of workbooks so the
    // total is negligible, but a long-lived WASM session with many creates/
    // deletes will accumulate.  Fixing requires UserModel to accept `String`
    // (an upstream API change in the base crate).
    // let name: &'static str = format!("Workbook {}", registry.len() + 1).leak();

    let max_n = registry
        .values()
        .filter_map(|meta| {
            meta.name
                .strip_prefix("Workbook ")
                .and_then(|s| s.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0);

    let name: &'static str = format!("Workbook {}", max_n + 1).leak();
    let uuid = crate::util::new_uuid();
    #[allow(clippy::expect_used)]
    let model = UserModel::new_empty(name, "en", "UTC", "en").expect("Failed to create new model");
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Persist an already-constructed model under a fresh UUID and set it as selected.
///
/// Used when the user uploads a file — the model is already in memory; we just
/// need to register and persist it.
pub fn create_new_from(model: UserModel<'static>) -> (String, UserModel<'static>) {
    let uuid = crate::util::new_uuid();
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Remove a workbook from localStorage and the registry.
pub fn delete(uuid: &str) {
    LocalStorage::delete(uuid);
    let mut registry = load_registry();
    registry.remove(uuid);
    save_registry(&registry);
    if get_selected_uuid().as_deref() == Some(uuid) {
        LocalStorage::delete(SELECTED_KEY);
    }
}
