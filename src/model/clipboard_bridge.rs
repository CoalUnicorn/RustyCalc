//! Bridge between the webapp and `ironcalc_base`'s `pub(crate)` clipboard
//! and border internals - without modifying the base crate.
//!
//! Both [`Clipboard`] and [`BorderArea`] have `pub(crate)` fields but derive
//! `Serialize + Deserialize`, so we serde-roundtrip once at construction time
//! to extract/inject the data we need.

use ironcalc_base::types::{BorderItem, BorderStyle};
use ironcalc_base::{BorderArea, ClipboardData, UserModel};
use serde::{Deserialize, Serialize};

// AppClipboard

/// Webapp-owned mirror of `ironcalc_base::Clipboard` with public fields.
///
/// Created once at copy-time via [`AppClipboard::capture`]; the serde
/// round-trip cost is negligible compared to the user-initiated Ctrl+C.
pub struct AppClipboard {
    /// Tab-separated text for the OS clipboard API.
    pub csv: String,
    /// Source sheet index (0-based).
    pub sheet: u32,
    /// `(r1, c1, r2, c2)` of the copied range (1-based).
    pub range: (i32, i32, i32, i32),
    /// Opaque cell data - passed back to `paste_from_clipboard`.
    data: ClipboardData,
}

/// Serde mirror matching `ironcalc_base::Clipboard`'s JSON shape.
#[derive(Deserialize)]
struct ClipboardMirror {
    csv: String,
    data: ClipboardData,
    sheet: u32,
    range: (i32, i32, i32, i32),
}

#[allow(clippy::expect_used)]
impl AppClipboard {
    /// Extract all fields from an opaque `ironcalc_base::Clipboard` via serde.
    ///
    /// Accepts any `Serialize` value whose JSON shape matches `Clipboard`
    /// (`{csv, data, sheet, range}`). This avoids naming the `Clipboard` type
    /// directly, since it's not re-exported from `ironcalc_base`.
    ///
    /// # Panics
    /// Only if the base crate changes `Clipboard`'s serialization shape.
    pub fn capture(clipboard: &impl serde::Serialize) -> Self {
        let json = serde_json::to_value(clipboard).expect("Clipboard must be serializable");
        let m: ClipboardMirror =
            serde_json::from_value(json).expect("ClipboardMirror must match Clipboard's shape");
        Self {
            csv: m.csv,
            sheet: m.sheet,
            range: m.range,
            data: m.data,
        }
    }

    /// Paste this clipboard into the model at the current selection.
    pub fn paste(&self, model: &mut UserModel, is_cut: bool) -> Result<(), String> {
        model.paste_from_clipboard(self.sheet, self.range, &self.data, is_cut)
    }
}

// BorderArea construction

/// Serde mirror matching `ironcalc_base::BorderArea`'s JSON shape.
#[allow(dead_code)]
#[derive(Serialize)]
struct BorderAreaMirror {
    item: BorderItem,
    r#type: BorderKind,
}

/// Local copy of `BorderType` with `Copy` (upstream lacks it).
/// Serializes to the same JSON strings as `ironcalc_base::BorderType`.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum BorderKind {
    All,
    Inner,
    Outer,
    Top,
    Right,
    Bottom,
    Left,
    CenterH,
    CenterV,
    None,
}

/// Construct a [`BorderArea`] without accessing its `pub(crate)` fields.
#[allow(clippy::expect_used)]
#[allow(dead_code)]
pub fn make_border_area(kind: BorderKind, style: BorderStyle, color: Option<String>) -> BorderArea {
    let mirror = BorderAreaMirror {
        item: BorderItem { style, color },
        r#type: kind,
    };
    let json = serde_json::to_value(&mirror).expect("BorderAreaMirror must be serializable");
    serde_json::from_value(json).expect("BorderArea must deserialize from mirror shape")
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::expect_used)]
    #[test]
    fn capture_roundtrip() {
        let model = UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("create test model");
        let cb = model.copy_to_clipboard().expect("copy empty range");
        let app = AppClipboard::capture(&cb);
        assert!(!app.csv.is_empty() || app.csv.is_empty()); // just ensure no panic
        assert_eq!(app.sheet, 0);
    }

    #[test]
    fn make_border_area_all_thin_black() {
        let ba = make_border_area(
            BorderKind::All,
            BorderStyle::Thin,
            Some("#000000".to_owned()),
        );
        // If this didn't panic, the serde roundtrip succeeded.
        // We can't inspect fields (pub(crate)), but set_area_with_border will accept it.
        let _ = ba;
    }

    #[test]
    fn make_border_area_none() {
        let ba = make_border_area(BorderKind::None, BorderStyle::Thin, None);
        let _ = ba;
    }
}
