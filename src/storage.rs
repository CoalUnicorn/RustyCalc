use base64::{engine::general_purpose::STANDARD, Engine};
use gloo_storage::{LocalStorage, Storage};
use ironcalc_base::UserModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

// localStorage key constants mirroring storage.ts
const SELECTED_KEY: &str = "selected";
const MODELS_KEY: &str = "models";

/// A 16-byte UUID v4 identifier for a workbook.
///
/// `Copy` with zero heap allocation — unlike `String`, passing by value costs nothing.
/// Serializes as a hyphenated UUID string (`"550e8400-e29b-41d4-a716-446655440000"`)
/// so localStorage keys remain human-readable and backward-compatible.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WorkbookId([u8; 16]);

impl WorkbookId {
    /// Generate a UUID v4 using `window.crypto.getRandomValues` (CSPRNG).
    #[allow(clippy::expect_used)]
    pub fn new() -> Self {
        let mut buf = [0u8; 16];
        let crypto = leptos::prelude::window()
            .crypto()
            .expect("crypto must be available");
        crypto
            .get_random_values_with_u8_array(&mut buf)
            .expect("getRandomValues must not fail for 16 bytes");
        buf[6] = (buf[6] & 0x0f) | 0x40; // version 4
        buf[8] = (buf[8] & 0x3f) | 0x80; // variant 10xx
        Self(buf)
    }
}

impl fmt::Display for WorkbookId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = &self.0;
        write!(
            f,
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3],
            b[4], b[5],
            b[6], b[7],
            b[8], b[9],
            b[10], b[11], b[12], b[13], b[14], b[15],
        )
    }
}

#[derive(Debug)]
pub struct WorkbookIdParseError;

impl fmt::Display for WorkbookIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid UUID string")
    }
}

impl FromStr for WorkbookId {
    type Err = WorkbookIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Accept "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx" (36 chars with dashes).
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return Err(WorkbookIdParseError);
        }
        let mut buf = [0u8; 16];
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| WorkbookIdParseError)?;
        }
        Ok(Self(buf))
    }
}

impl Serialize for WorkbookId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for WorkbookId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse()
            .map_err(|_| serde::de::Error::custom("invalid UUID"))
    }
}

/// Log a storage error to the browser console and discard the `Err`.
/// Used in place of bare `.ok()` so silent failures become visible in DevTools.
fn log_err<E: std::fmt::Display>(result: Result<(), E>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[rustycalc storage] {ctx}: {e}").into());
    }
}

/// Per-workbook metadata stored in the "models" registry.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkbookMeta {
    pub name: String,
    #[serde(default)]
    pub group: WorkbookGroup,
    /// Last-modified timestamp (ms since epoch). Used for sort-by-recent.
    #[serde(default)]
    pub modified: f64,
}

#[derive(Clone, Default, Debug, PartialOrd, Ord, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "Option<String>", into = "Option<String>")]
pub enum WorkbookGroup {
    Named(String),
    #[default]
    Ungrouped,
}

impl From<Option<String>> for WorkbookGroup {
    fn from(s: Option<String>) -> Self {
        match s {
            Some(name) => WorkbookGroup::Named(name),
            None => WorkbookGroup::Ungrouped,
        }
    }
}

impl From<WorkbookGroup> for Option<String> {
    fn from(g: WorkbookGroup) -> Self {
        match g {
            WorkbookGroup::Named(name) => Some(name),
            WorkbookGroup::Ungrouped => None,
        }
    }
}

/// Update the group label for a workbook in the registry.
pub fn update_group(uuid: &WorkbookId, group: WorkbookGroup) {
    let mut registry = load_registry();
    if let Some(meta) = registry.get_mut(uuid) {
        meta.group = group;
    }
    save_registry(&registry);
}

pub fn update_name(uuid: &WorkbookId, name: &str) {
    let mut registry = load_registry();
    if let Some(meta) = registry.get_mut(uuid) {
        meta.name = name.to_string();
    }
    save_registry(&registry);
}

// Registry helpers

/// Load the UUID->metadata registry from localStorage.
pub fn load_registry() -> HashMap<WorkbookId, WorkbookMeta> {
    LocalStorage::get(MODELS_KEY).unwrap_or_default()
}

fn save_registry(registry: &HashMap<WorkbookId, WorkbookMeta>) {
    log_err(LocalStorage::set(MODELS_KEY, registry), "save registry");
}

// Selection helpers

/// Return the UUID of the currently selected workbook, if set.
pub fn get_selected_uuid() -> Option<WorkbookId> {
    LocalStorage::get(SELECTED_KEY).ok()
}

/// Persist the active workbook UUID so it survives page reloads.
pub fn set_selected_uuid(uuid: &WorkbookId) {
    log_err(LocalStorage::set(SELECTED_KEY, uuid), "set selected uuid");
}

// Core CRUD

/// Serialize `model` to bytes, base64-encode, and write to localStorage.
/// Also refreshes the workbook's entry in the metadata registry.
pub fn save(uuid: &WorkbookId, model: &UserModel) {
    let bytes = model.to_bytes();
    let encoded = STANDARD.encode(&bytes);
    log_err(
        LocalStorage::set(uuid.to_string(), encoded),
        "save model bytes",
    );

    let mut registry = load_registry();

    registry.insert(
        *uuid,
        WorkbookMeta {
            name: model.get_name(),
            group: registry
                .get(uuid)
                .map(|m| m.group.clone())
                .unwrap_or_default(),
            modified: crate::perf::now(),
        },
    );

    save_registry(&registry);
}

/// Decode and deserialize a model from localStorage.
/// Returns `None` if the key is absent or the bytes are corrupt.
/// Logs a console warning for decode/parse failures so silent data loss is visible.
pub fn load(uuid: &WorkbookId) -> Option<UserModel<'static>> {
    let encoded: String = LocalStorage::get(&uuid.to_string()).ok()?;
    let bytes = match STANDARD.decode(encoded) {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[rustycalc storage] load {uuid}: base64 decode failed: {e}").into(),
            );
            return None;
        }
    };
    // "en" is 'static, so the returned UserModel<'static> lifetime is satisfied.
    match UserModel::from_bytes(&bytes, "en") {
        Ok(m) => Some(m),
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[rustycalc storage] load {uuid}: model parse failed: {e}").into(),
            );
            None
        }
    }
}

/// Load the previously selected workbook, falling back to the first available.
/// Returns `None` only when localStorage is completely empty.
pub fn load_selected() -> Option<(WorkbookId, UserModel<'static>)> {
    // Try the explicitly selected UUID first.
    if let Some(uuid) = get_selected_uuid() {
        if let Some(model) = load(&uuid) {
            return Some((uuid, model));
        }
    }

    // Fall back to the lexicographically first UUID that yields a valid model.
    // Sorting ensures a stable, repeatable result regardless of HashMap iteration order.
    let registry = load_registry();
    let mut uuids: Vec<WorkbookId> = registry.keys().cloned().collect();
    uuids.sort();
    for uuid in &uuids {
        if let Some(model) = load(uuid) {
            set_selected_uuid(uuid);
            return Some((*uuid, model));
        }
    }

    None
}

/// Create a fresh blank workbook, persist it, set it as selected.
/// The workbook is named "Workbook N" where N is one more than the current registry size.
pub fn create_new() -> (WorkbookId, UserModel<'static>) {
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
    let uuid = WorkbookId::new();
    #[allow(clippy::expect_used)]
    let model = UserModel::new_empty(name, "en", "UTC", "en").expect("Failed to create new model");
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Persist an already-constructed model under a fresh UUID and set it as selected.
///
/// Used when the user uploads a file - the model is already in memory; we just
/// need to register and persist it.
#[allow(dead_code)]
pub fn create_new_from(model: UserModel<'static>) -> (WorkbookId, UserModel<'static>) {
    let uuid = WorkbookId::new();
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Remove a workbook from localStorage and the registry.
pub fn delete(uuid: &WorkbookId) {
    LocalStorage::delete(uuid.to_string());
    let mut registry = load_registry();
    registry.remove(uuid);
    save_registry(&registry);
    if get_selected_uuid() == Some(*uuid) {
        LocalStorage::delete(SELECTED_KEY);
    }
}
