//! Bridge between the webapp and `ironcalc_base`'s `pub(crate)` clipboard
//! and border internals - without modifying the base crate.
//!
//! Both [`Clipboard`] and [`BorderArea`] have `pub(crate)` fields but derive
//! `Serialize + Deserialize`, so we serde-roundtrip once at construction time
//! to extract/inject the data we need.

use ironcalc_base::types::{BorderItem, BorderStyle};
use ironcalc_base::{BorderArea, ClipboardData, UserModel};

use crate::coord::CellArea;

use super::frontend_model::FrontendModel;
use serde::{Deserialize, Serialize};

// PasteMode

/// Whether a paste operation originates from a copy or a cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMode {
    Copy,
    Cut,
}

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
    /// Copied cell range (1-based).
    pub range: CellArea,
    /// Opaque cell data - passed back to `paste_from_clipboard`.
    data: ClipboardData,
}

/// Serde mirror matching `ironcalc_base::Clipboard`'s JSON shape.
#[derive(Deserialize)]
struct ClipboardMirror {
    csv: String,
    data: ClipboardData,
    sheet: u32,
    /// ironcalc serialises this as a `(r1, c1, r2, c2)` tuple.
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
            range: m.range.into(),
            data: m.data,
        }
    }

    /// Paste this clipboard into the model at the current selection.
    ///
    /// Tiling rules (matching Excel / Google Sheets behaviour):
    /// - **Exact multiples** → tiles the source to fill the destination.
    /// - **Non-multiple destination** → pastes once from the top-left corner.
    /// - **Cut** → never tiles; always pastes once.
    pub fn paste(&self, model: &mut UserModel, mode: PasteMode) -> Result<(), String> {
        if mode == PasteMode::Cut {
            return model.paste_from_clipboard(self.sheet, self.range.as_tuple(), &self.data, true);
        }

        let src = self.range;
        let dst = CellArea::from(model.get_selected_view().range);

        if let Some((row_reps, col_reps)) = dst.tile_reps_of(src) {
            for tr in 0..row_reps {
                for tc in 0..col_reps {
                    let row = dst.r1 + (tr * src.height()) as i32;
                    let col = dst.c1 + (tc * src.width()) as i32;
                    model.set_selected_cell(row, col)?;
                    model.paste_from_clipboard(self.sheet, src.as_tuple(), &self.data, false)?;
                }
            }

            model.set_selected_area(dst);
            return Ok(());
        }

        model.paste_from_clipboard(self.sheet, src.as_tuple(), &self.data, false)
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

    // CellRange::tile_reps_of

    #[test]
    fn tile_reps_single_cell_into_range() {
        let src = CellArea {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1,
        };
        let dst = CellArea {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 4,
        };
        assert_eq!(dst.tile_reps_of(src), Some((3, 4)));
    }

    #[test]
    fn tile_reps_exact_multiple() {
        let src = CellArea {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 3,
        };
        let dst = CellArea {
            r1: 1,
            c1: 1,
            r2: 4,
            c2: 6,
        };
        assert_eq!(dst.tile_reps_of(src), Some((2, 2)));
    }

    #[test]
    fn tile_reps_non_multiple_returns_none() {
        let src = CellArea {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        let dst = CellArea {
            r1: 1,
            c1: 1,
            r2: 3,
            c2: 3,
        };
        assert_eq!(dst.tile_reps_of(src), None);
    }

    #[test]
    fn tile_reps_same_size_returns_none() {
        let src = CellArea {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        };
        assert_eq!(src.tile_reps_of(src), None);
    }

    // AppClipboard::capture roundtrip

    #[allow(clippy::expect_used)]
    #[test]
    fn capture_roundtrip() {
        let model = UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("create test model");
        let cb = model.copy_to_clipboard().expect("copy empty range");
        let app = AppClipboard::capture(&cb);
        assert_eq!(app.sheet, 0);
    }

    // BorderArea construction

    #[test]
    fn make_border_area_all_thin_black() {
        let ba = make_border_area(
            BorderKind::All,
            BorderStyle::Thin,
            Some("#000000".to_owned()),
        );
        // If this didn't panic, the serde roundtrip succeeded.
        let _ = ba;
    }

    #[test]
    fn make_border_area_none() {
        let ba = make_border_area(BorderKind::None, BorderStyle::Thin, None);
        let _ = ba;
    }
}
