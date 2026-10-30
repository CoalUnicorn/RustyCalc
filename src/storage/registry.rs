//! Workbook metadata registry and active-selection persistence.
//!
//! IndexedDB owns the registry. The browser bridge caches committed metadata.

use gloo_storage::{LocalStorage, Storage};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{SELECTED_KEY, WorkbookId, database, log_err};

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
/// Strips C0 control characters (U+0000-U+001F, U+007F) and bidi override
/// characters (U+200E-U+200F, U+202A-U+202E, U+2066-U+2069) that could
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

    if cleaned.chars().count() <= 128 {
        cleaned
    } else {
        // Truncate at a char boundary.
        cleaned.chars().take(128).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_name;

    #[test]
    fn sanitize_name_keeps_128_multibyte_chars() {
        let name = "é".repeat(128);
        assert_eq!(sanitize_name(&name), name);
    }

    #[test]
    fn sanitize_name_truncates_by_chars_not_bytes() {
        let name = "é".repeat(129);
        assert_eq!(sanitize_name(&name).chars().count(), 128);
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
pub async fn update_group(uuid: &WorkbookId, group: WorkbookGroup) -> Result<(), String> {
    update_metadata(uuid, serde_json::json!({ "group": group })).await
}

pub async fn promote_from_shared(uuid: &WorkbookId) -> Result<(), String> {
    update_metadata(uuid, serde_json::json!({ "shared_from_link": false })).await
}

async fn update_metadata(uuid: &WorkbookId, patch: serde_json::Value) -> Result<(), String> {
    database::update(&uuid.to_string(), &patch.to_string())
        .await
        .map(|_| ())
        .map_err(|error| {
            format!(
                "Cannot update workbook metadata: {}",
                database::error(error)
            )
        })
}

/// Read the committed metadata cache. Names are sanitized before display.
pub fn load_registry() -> HashMap<WorkbookId, WorkbookMeta> {
    let raw: HashMap<WorkbookId, WorkbookMeta> =
        serde_json::from_str(&database::registry()).unwrap_or_default();
    raw.into_iter()
        .map(|(uuid, meta)| {
            (
                uuid,
                WorkbookMeta {
                    name: sanitize_name(&meta.name),
                    ..meta
                },
            )
        })
        .collect()
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
