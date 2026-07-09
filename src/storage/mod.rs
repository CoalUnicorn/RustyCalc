//! Workbook persistence: localStorage CRUD, metadata registry, and URL sharing.
//!
//! Split across [`registry`] (metadata + selection), [`persist`] (binary model
//! CRUD), and [`share`] (base64 share-URL codec). This root holds the shared
//! [`WorkbookId`] primitive plus the helpers and consts every submodule needs.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

mod persist;
mod registry;
mod share;

pub use persist::*;
pub use registry::*;
pub use share::*;

// localStorage key for the currently-selected workbook UUID. Shared: registry's
// selection helpers write it; persist's `delete` clears it when the active
// workbook is removed. Private const — visible to submodules via `super::`.
const SELECTED_KEY: &str = "selected";

/// Locale used for `UserModel` construction throughout storage.
/// IronCalc's parser uses this for function-name resolution, number
/// formatting, and date parsing. Shared by persist (create/load) and share
/// (accept/verify) so the round-trip stays consistent.
const LOCALE: &str = "en";

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
        let crypto_ok = web_sys::window().and_then(|w| w.crypto().ok());
        if let Some(crypto) = crypto_ok
            && crypto.get_random_values_with_u8_array(&mut buf).is_ok()
        {
            buf[6] = (buf[6] & 0x0f) | 0x40;
            buf[8] = (buf[8] & 0x3f) | 0x80;
            return Self(buf);
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
            b[0],
            b[1],
            b[2],
            b[3],
            b[4],
            b[5],
            b[6],
            b[7],
            b[8],
            b[9],
            b[10],
            b[11],
            b[12],
            b[13],
            b[14],
            b[15],
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
pub(crate) fn log_err<E: std::fmt::Display>(result: Result<(), E>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[rustycalc storage] {ctx}: {e}").into());
    }
}
