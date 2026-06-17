//! Binary model persistence: serialize a `UserModel` to a magic-tagged,
//! base64 localStorage entry and back, keyed by workbook UUID.

use base64::{Engine, engine::general_purpose::STANDARD};
use ironcalc_base::UserModel;

use super::registry::{
    WorkbookMeta, get_selected_uuid, load_registry, sanitize_name, save_registry, set_selected_uuid,
};
use super::{LOCALE, SELECTED_KEY, WorkbookId, log_err};

use gloo_storage::{LocalStorage, Storage};

/// Magic bytes prepended to every localStorage entry so we can fast-reject
/// non-RustyCalc blobs before the bitcode parser sees them.
const STORAGE_MAGIC: &[u8; 4] = b"RCAL";

/// Current storage format version. Bump when IronCalc schema changes in a
/// way that breaks backward compatibility with existing stored data.
const STORAGE_VERSION: u8 = 1;

/// Maximum decoded byte size for localStorage entries.
/// 5 MB is generous for a spreadsheet — a 30-sheet book with formulas
/// typically fits in 50-200 KB.
pub const MAX_STORED_BYTES: usize = 5_000_000;

/// Serialize `model` to bytes, base64-encode, and write to localStorage.
/// Also refreshes the workbook's entry in the metadata registry.
pub fn save(uuid: &WorkbookId, model: &UserModel) {
    let model_bytes = model.to_bytes();
    let mut bytes = Vec::with_capacity(5 + model_bytes.len());
    bytes.extend_from_slice(STORAGE_MAGIC);
    bytes.push(STORAGE_VERSION);
    bytes.extend_from_slice(&model_bytes);
    let encoded = STANDARD.encode(&bytes);
    log_err(
        LocalStorage::set(uuid.to_string(), encoded),
        "save model bytes",
    );

    let mut registry = load_registry();

    registry.insert(
        *uuid,
        WorkbookMeta {
            name: sanitize_name(&model.get_name()),
            group: registry
                .get(uuid)
                .map(|m| m.group.clone())
                .unwrap_or_default(),
            modified: crate::perf::now(),
            // Preserve the shared_from_link flag if it was set, don't clear
            // it on save — it gets cleared explicitly by promote_from_shared.
            shared_from_link: registry
                .get(uuid)
                .map(|m| m.shared_from_link)
                .unwrap_or(false),
        },
    );

    save_registry(&registry);
}

/// Decode and deserialize a model from localStorage.
/// Returns `None` if the key is absent or the bytes are corrupt.
/// Logs a console warning for decode/parse failures so silent data loss is visible.
pub fn load(uuid: &WorkbookId) -> Option<UserModel<'static>> {
    let encoded: String = LocalStorage::get(uuid.to_string()).ok()?;
    let bytes = match STANDARD.decode(encoded) {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[rustycalc storage] load {uuid}: base64 decode failed: {e}").into(),
            );
            return None;
        }
    };

    // Size ceiling — reject obviously oversized entries before parsing.
    if bytes.len() > MAX_STORED_BYTES {
        web_sys::console::warn_1(
            &format!(
                "[rustycalc storage] load {uuid}: {} bytes exceeds limit {MAX_STORED_BYTES}",
                bytes.len()
            )
            .into(),
        );
        return None;
    }

    // Check magic header.
    if bytes.len() < 5 || &bytes[..4] != STORAGE_MAGIC {
        web_sys::console::warn_1(
            &format!("[rustycalc storage] load {uuid}: bad magic — not a RustyCalc workbook")
                .into(),
        );
        return None;
    }

    let version = bytes[4];
    if version != STORAGE_VERSION {
        web_sys::console::warn_1(
            &format!(
                "[rustycalc storage] load {uuid}: version mismatch (got {version}, current {STORAGE_VERSION}) — schema may have changed"
            )
            .into(),
        );
        return None;
    }

    let model_bytes = &bytes[5..];
    // LOCALE is 'static, so the returned UserModel<'static> lifetime is satisfied.
    match UserModel::from_bytes(model_bytes, LOCALE) {
        Ok(mut m) => {
            // Sanitize the model's internal name — workbooks imported via shared URL
            // or saved before the sanitizer was added may carry C0 controls or bidi
            // overrides in their name. We apply sanitization on load so every display
            // path (sidebar, confirm dialogs, file bar) sees clean names.
            m.set_name(&sanitize_name(&m.get_name()));
            Some(m)
        }
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
    if let Some(uuid) = get_selected_uuid()
        && let Some(model) = load(&uuid)
    {
        return Some((uuid, model));
    }

    // Fall back to the lexicographically first UUID that yields a valid model.
    // Sorting ensures a stable, repeatable result regardless of HashMap iteration order.
    // Capped at MAX_STORAGE_FALLBACK attempts so a poisoned registry full of
    // corrupted entries doesn't wedge startup (see security audit 2026-05-26).
    const MAX_STORAGE_FALLBACK: usize = 10;
    let registry = load_registry();
    let mut uuids: Vec<WorkbookId> = registry.keys().cloned().collect();
    uuids.sort();
    let mut tried = 0;
    for uuid in &uuids {
        if let Some(model) = load(uuid) {
            set_selected_uuid(uuid);
            return Some((*uuid, model));
        }
        tried += 1;
        if tried >= MAX_STORAGE_FALLBACK {
            web_sys::console::warn_1(
                &format!(
                    "[rustycalc storage] load_selected: {tried} fallback attempts exhausted, {remaining} entries skipped",
                    remaining = uuids.len().saturating_sub(tried)
                )
                .into(),
            );
            break;
        }
    }

    None
}

/// Create a fresh blank workbook, persist it, set it as selected.
/// The workbook is named "Workbook N" where N is one more than the current registry size.
pub fn create_new() -> (WorkbookId, UserModel<'static>) {
    let registry = load_registry();
    // `leak()` gives a `&'static str` so UserModel<'static> can borrow it.
    // Note: each call leaks a small heap allocation that is never reclaimed.
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
    let model = UserModel::new_empty(name, LOCALE, "UTC", LOCALE)
        .unwrap_or_else(|e| panic!("new_empty failed with builtin locale: {e:?}"));
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Where a freshly-registered workbook came from. A file import is
/// user-provided and trusted; a share link is quarantined (badged) until the
/// first edit promotes it — so only `ShareLink` sets `shared_from_link` (#19).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkbookOrigin {
    ShareLink,
    FileImport,
}

/// Persist an already-constructed model under a fresh UUID and set it as selected.
///
/// Used when the user uploads a file or accepts a shared link — the model is
/// already in memory; we just register and persist it. `origin` drives the
/// shared-link quarantine badge.
/// Safety: caller must ensure model originated from a trusted source
/// (validated by both callers upstream — `share_verify.rs` verifies the
/// shared payload, `workbook.rs` uploads from a user-provided file).
pub fn create_new_from(
    model: UserModel<'static>,
    origin: WorkbookOrigin,
) -> (WorkbookId, UserModel<'static>) {
    let uuid = WorkbookId::new();
    save(&uuid, &model);
    // Badge share-link ingests so the sidebar can quarantine them; a file
    // import is trusted and stays unbadged. The flag clears on the first edit.
    if origin == WorkbookOrigin::ShareLink {
        let mut registry = load_registry();
        if let Some(meta) = registry.get_mut(&uuid) {
            meta.shared_from_link = true;
        }
        save_registry(&registry);
    }
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Remove the quarantine badge from a shared-from-link workbook.
/// Call after the first user edit to promote it to a regular workbook.
pub fn promote_from_shared(uuid: &WorkbookId) {
    let mut registry = load_registry();
    if let Some(meta) = registry.get_mut(uuid)
        && meta.shared_from_link
    {
        meta.shared_from_link = false;
        save_registry(&registry);
    }
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
