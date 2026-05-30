//! URL Sharing
//!
//! Encodes the entire workbook into a URL hash fragment so a recipient can open
//! a copy with no server round-trip. Raw model bytes → base64url (no padding)
//! keeps the result safe for `#share=…` without percent-encoding. The 30 KB
//! raw-byte ceiling keeps the resulting link well below practical browser URL
//! limits (~64 KB on Chrome / ~32 KB on most others when shared via chat apps).

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ironcalc_base::UserModel;

use crate::verify;

use super::LOCALE;

/// Maximum raw byte size for share-URL encoding.
/// 30 KB raw → ~40 KB base64url, comfortably within practical hash-fragment
/// limits across browsers and link-preview tools.
pub const MAX_SHARE_BYTES: usize = 30_000;

/// Maximum encoded (base64url) length for incoming share URLs.
/// ceil(MAX_SHARE_BYTES × 4/3) — a guard before decode so attackers can't
/// feed arbitrarily large payloads into the parser.
const MAX_SHARE_ENCODED: usize = MAX_SHARE_BYTES * 4 / 3 + 4;

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
            web_sys::console::warn_1(&format!("[rustycalc sharing] URL decode failed: {e}").into());
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
            web_sys::console::warn_1(&"[rustycalc sharing] unknown share payload version".into());
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
    verify::decode_with_consent(hash, word).map_err(|e| e.to_string())?;
    UserModel::from_bytes(bytes, LOCALE).map_err(|e| {
        web_sys::console::warn_1(
            &format!("[rustycalc sharing] model parse failed after verification: {e}").into(),
        );
        format!("Failed to load shared workbook: {e}")
    })
}
