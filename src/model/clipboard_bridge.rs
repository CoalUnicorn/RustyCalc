//! Serde-roundtrip bridge to access `ironcalc_base`'s `pub(crate)`
//! clipboard and border fields without modifying the base crate.

use ironcalc_base::types::{BorderItem, BorderStyle};
use ironcalc_base::{BorderArea, ClipboardData, UserModel};

use crate::model::Navigator;

use crate::coord::CellArea;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMode {
    Copy,
    Cut,
}

/// Public mirror of `ironcalc_base::Clipboard`. Created at copy-time
/// via `capture()`; serde round-trip extracts the `pub(crate)` fields.
pub struct AppClipboard {
    pub csv: String,
    pub sheet: u32,
    pub range: CellArea,
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

    /// Tiles source to fill destination if dimensions are exact multiples,
    /// otherwise pastes once. Cut never tiles.
    pub fn paste(&self, model: &mut UserModel, mode: PasteMode) -> Result<(), String> {
        if mode == PasteMode::Cut {
            return model.paste_from_clipboard(self.sheet, self.range.as_tuple(), &self.data, true);
        }

        let src = self.range;
        let dst = CellArea::from(model.get_selected_view().range);

        if let Some((row_reps, col_reps)) = dst.tile_reps_of(src) {
            for tr in 0..row_reps {
                for tc in 0..col_reps {
                    let row = dst.r1 + (tr * src.height());
                    let col = dst.c1 + (tc * src.width());
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

#[allow(dead_code)]
#[derive(Serialize)]
struct BorderAreaMirror {
    item: BorderItem,
    r#type: BorderKind,
}

/// Local `Copy` mirror of `ironcalc_base::BorderType`.
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

/// Construct a `BorderArea` via serde roundtrip (fields are `pub(crate)`).
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

