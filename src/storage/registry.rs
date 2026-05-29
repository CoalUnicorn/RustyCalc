//! Workbook metadata registry and active-selection persistence.
//!
//! The registry is a `UUID -> WorkbookMeta` map stored under one localStorage
//! key; model bytes themselves live elsewhere (see [`super::persist`]).

use gloo_storage::{LocalStorage, Storage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{SELECTED_KEY, WorkbookId, log_err};

// localStorage key for the UUID->metadata map, mirroring storage.ts.
const MODELS_KEY: &str = "models";

/// Per-workbook metadata stored in the "models" registry.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WorkbookMeta {
    pub name: String,
    #[serde(default)]
    pub group: WorkbookGroup,
    /// Last-modified timestamp (ms since epoch). Used for sort-by-recent.
    #[serde(default)]
    pub modified: f64,
    /// True if this workbook was ingested from a shared link (#share=).
    /// Displayed with a "Shared" badge in the sidebar until the user
    /// explicitly promotes it by editing.
    #[serde(default)]
    pub shared_from_link: bool,
}

/// Clamp and sanitize a workbook name for safe display in the sidebar.
///
/// Strips C0 control characters (U+0000–U+001F, U+007F) and bidi override
/// characters (U+200E–U+200F, U+202A–U+202E, U+2066–U+2069) that could
/// confuse UI layout. Truncates to 128 characters so an attacker can't
/// inject a 1 MB name through a poisoned localStorage registry.
///
/// Applied at every boundary where untrusted names cross into rendering:
/// on save/rename input, on `load_registry()` deserialization, and again at
/// render-time in the sidebar as a defense-in-depth backstop.
pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            // Reject C0 controls
            if cp <= 0x1F || cp == 0x7F {
                return false;
            }
            // Reject bidi overrides
            if (0x200E..=0x200F).contains(&cp)
                || (0x202A..=0x202E).contains(&cp)
                || (0x2066..=0x2069).contains(&cp)
            {
                return false;
            }
            true
        })
        .collect();

    if cleaned.len() <= 128 {
        cleaned
    } else {
        // Truncate at a char boundary.
        cleaned.chars().take(128).collect()
    }
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
        meta.name = sanitize_name(name);
    }
    save_registry(&registry);
}

// Registry helpers

/// Load the UUID->metadata registry from localStorage.
///
/// Names are sanitized on the way out so any pre-sanitizer or share-imported
/// entry can't leak control chars or bidi overrides into the sidebar.
pub fn load_registry() -> HashMap<WorkbookId, WorkbookMeta> {
    let raw: HashMap<WorkbookId, WorkbookMeta> = LocalStorage::get(MODELS_KEY).unwrap_or_default();
    raw.into_iter()
        .map(|(uuid, meta)| {
            let cleaned = WorkbookMeta {
                name: sanitize_name(&meta.name),
                ..meta
            };
            (uuid, cleaned)
        })
        .collect()
}

/// Persist the registry map. `pub(super)` so [`super::persist`] can refresh
/// metadata after a model write without exposing it as crate-public API.
pub(super) fn save_registry(registry: &HashMap<WorkbookId, WorkbookMeta>) {
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
