use crate::verify;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use gloo_storage::{LocalStorage, Storage};
use ironcalc_base::UserModel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

// localStorage key constants mirroring storage.ts
const SELECTED_KEY: &str = "selected";
const MODELS_KEY: &str = "models";

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

/// A 16-byte UUID v4 identifier for a workbook.
///
/// `Copy` with zero heap allocation — unlike `String`, passing by value costs nothing.
/// Serializes as a hyphenated UUID string (`"550e8400-e29b-41d4-a716-446655440000"`)
/// so localStorage keys remain human-readable and backward-compatible.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct WorkbookId([u8; 16]);

impl WorkbookId {
    /// Generate a UUID v4 using `window.crypto.getRandomValues` (CSPRNG).
    /// Falls back to `Math.random()` if crypto is unavailable (private-mode,
    /// sandboxed iframe). The fallback has lower entropy but is sufficient
    /// for localStorage key uniqueness within a single origin.
    #[allow(clippy::expect_used)]
    pub fn new() -> Self {
        let mut buf = [0u8; 16];
        // Try CSPRNG first. `window().crypto()` returns Result<Crypto, JsValue>;
        // in private-mode/iframe contexts it may fail — fall back to Math.random().
        let crypto_ok = web_sys::window()
            .and_then(|w| w.crypto().ok());
        if let Some(crypto) = crypto_ok {
            if crypto.get_random_values_with_u8_array(&mut buf).is_ok() {
                buf[6] = (buf[6] & 0x0f) | 0x40;
                buf[8] = (buf[8] & 0x3f) | 0x80;
                return Self(buf);
            }
        }
        // Fallback: Math.random() — lower entropy but doesn't panic.
        for byte in &mut buf {
            *byte = (js_sys::Math::random() * 256.0) as u8;
        }
        buf[6] = (buf[6] & 0x0f) | 0x40;
        buf[8] = (buf[8] & 0x3f) | 0x80;
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
            &format!("[rustycalc storage] load {uuid}: bad magic — not a RustyCalc workbook").into(),
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
    if let Some(uuid) = get_selected_uuid() {
        if let Some(model) = load(&uuid) {
            return Some((uuid, model));
        }
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
    let model = UserModel::new_empty(name, LOCALE, "UTC", LOCALE)
        .unwrap_or_else(|e| panic!("new_empty failed with builtin locale: {e:?}"));
    save(&uuid, &model);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Persist an already-constructed model under a fresh UUID and set it as selected.
///
/// Used when the user uploads a file - the model is already in memory; we just
/// need to register and persist it.
pub fn create_new_from(model: UserModel<'static>) -> (WorkbookId, UserModel<'static>) {
    let uuid = WorkbookId::new();
    save(&uuid, &model);
    // Mark as ingested from a shared link so the sidebar can show a
    // quarantine badge. The flag is cleared on the first user edit.
    let mut registry = load_registry();
    if let Some(meta) = registry.get_mut(&uuid) {
        meta.shared_from_link = true;
    }
    save_registry(&registry);
    set_selected_uuid(&uuid);
    (uuid, model)
}

/// Remove the quarantine badge from a shared-from-link workbook.
/// Call after the first user edit to promote it to a regular workbook.
pub fn promote_from_shared(uuid: &WorkbookId) {
    let mut registry = load_registry();
    if let Some(meta) = registry.get_mut(uuid) {
        if meta.shared_from_link {
            meta.shared_from_link = false;
            save_registry(&registry);
        }
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

// ============================================================================
// URL Sharing
// ============================================================================
//
// Encodes the entire workbook into a URL hash fragment so a recipient can open
// a copy with no server round-trip. Raw model bytes → base64url (no padding)
// keeps the result safe for `#share=…` without percent-encoding. The 30 KB
// raw-byte ceiling keeps the resulting link well below practical browser URL
// limits (~64 KB on Chrome / ~32 KB on most others when shared via chat apps).

/// Maximum raw byte size for share-URL encoding.
/// 30 KB raw → ~40 KB base64url, comfortably within practical hash-fragment
/// limits across browsers and link-preview tools.
pub const MAX_SHARE_BYTES: usize = 30_000;

/// Maximum encoded (base64url) length for incoming share URLs.
/// ceil(MAX_SHARE_BYTES × 4/3) — a guard before decode so attackers can't
/// feed arbitrarily large payloads into the parser.
const MAX_SHARE_ENCODED: usize = MAX_SHARE_BYTES * 4 / 3 + 4;

/// Locale used for UserModel construction throughout storage.
/// IronCalc's parser uses this for function-name resolution, number
/// formatting, and date parsing. Keep it consistent across create/load/import.
const LOCALE: &str = "en";

#[derive(Debug, Clone)]
pub enum ShareError {
    TooLarge { size_kb: usize },
}

/// A share payload that has been decoded but not yet consented to by the
/// recipient. Bitcode parsing is deferred until the user accepts so a
/// malicious crafted payload can't trigger expensive deserialization on
/// first paint.
#[derive(Clone)]
pub enum SharedLoad {
    /// v0 — no word verification. Recipient sees an accept/reject modal
    /// with size + source; bitcode parse happens on accept.
    PendingV0 { bytes: Vec<u8> },
    /// v1 — word verification required. Recipient must type the sender's
    /// word; on hash match the bytes are parsed.
    PendingV1 { hash: [u8; 32], bytes: Vec<u8> },
}

impl SharedLoad {
    /// Decoded payload size in bytes — used by the consent modal to show
    /// "X KB" so the recipient can sanity-check before accepting.
    pub fn size_bytes(&self) -> usize {
        match self {
            Self::PendingV0 { bytes } | Self::PendingV1 { bytes, .. } => bytes.len(),
        }
    }
}

/// Encode a model to a URL-safe base64 string for sharing.
/// Uses `URL_SAFE_NO_PAD` so the result drops into a hash fragment without
/// percent-encoding or trailing `=` padding.
///
/// word: optional verification word. If Some, the payload is wrapped with a
/// v1 version byte + SHA-256 hash prefix so the receiver must type the same
/// word before the workbook loads.
pub fn encode_for_share_url(model: &UserModel, word: Option<&str>) -> Result<String, ShareError> {
    let bytes = model.to_bytes();
    if bytes.len() > MAX_SHARE_BYTES {
        return Err(ShareError::TooLarge {
            size_kb: bytes.len() / 1024,
        });
    }
    let wrapped = verify::encode_with_version(word, &bytes);
    Ok(URL_SAFE_NO_PAD.encode(&wrapped))
}

/// Try to load a shared model from the URL hash fragment.
/// Returns `None` if no `#share=` parameter is present or decoding fails.
/// Warnings are logged so silent data loss is visible in the console.
pub fn load_shared_from_url() -> Option<SharedLoad> {
    let hash = leptos::prelude::window().location().hash().ok()?;
    let encoded = hash.strip_prefix("#share=")?;

    // Reject oversized payloads before decoding — prevents attackers from
    // feeding arbitrarily large base64 into the decoder/parser.
    if encoded.len() > MAX_SHARE_ENCODED {
        web_sys::console::warn_1(
            &format!(
                "[rustycalc sharing] encoded length {} exceeds limit {MAX_SHARE_ENCODED}",
                encoded.len()
            )
            .into(),
        );
        return None;
    }

    let bytes = match URL_SAFE_NO_PAD.decode(encoded) {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::warn_1(
                &format!("[rustycalc sharing] URL decode failed: {e}").into(),
            );
            return None;
        }
    };

    // Double-check decoded size matches our limit — base64 can decode to
    // a smaller payload than the encoded length suggested.
    if bytes.len() > MAX_SHARE_BYTES {
        web_sys::console::warn_1(
            &format!(
                "[rustycalc sharing] decoded {} bytes exceeds limit {MAX_SHARE_BYTES}",
                bytes.len()
            )
            .into(),
        );
        return None;
    }

    match verify::decode_payload(&bytes) {
        Some(verify::SharePayload::V0(payload)) => Some(SharedLoad::PendingV0 { bytes: payload }),
        Some(verify::SharePayload::V1 { hash, bytes }) => {
            Some(SharedLoad::PendingV1 { hash, bytes })
        }
        None => {
            web_sys::console::warn_1(
                &"[rustycalc sharing] unknown share payload version".into(),
            );
            None
        }
    }
}

/// Parse the staged V0 payload after the user accepts in the consent modal.
/// Splitting parse from decode is the security win: a crafted payload can't
/// run bitcode deserialization until the recipient has clicked Accept.
pub fn accept_shared_v0(bytes: &[u8]) -> Result<UserModel<'static>, String> {
    UserModel::from_bytes(bytes, LOCALE).map_err(|e| {
        web_sys::console::warn_1(
            &format!("[rustycalc sharing] model parse failed after consent: {e}").into(),
        );
        format!("Failed to load shared workbook: {e}")
    })
}

/// Try to verify and load a v1 shared workbook.
/// Call this after the user types the verification word.
///
/// Returns user-facing error strings — the caller (verification modal) displays
/// them directly. We use `String` rather than `VerifyError` so a post-verify
/// bitcode parse failure can carry its own message without being shoe-horned
/// into a verification-shaped error.
pub fn verify_and_load_shared(
    hash: &[u8; 32],
    word: &str,
    bytes: &[u8],
) -> Result<UserModel<'static>, String> {
    verify::verify_and_extract(hash, word).map_err(|e| e.to_string())?;
    UserModel::from_bytes(bytes, LOCALE).map_err(|e| {
        web_sys::console::warn_1(
            &format!("[rustycalc sharing] model parse failed after verification: {e}").into(),
        );
        format!("Failed to load shared workbook: {e}")
    })
}
