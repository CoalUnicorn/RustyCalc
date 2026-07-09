//! Top-level action enum and convenience constructors.

use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};

use crate::input::{
    edit::EditAction, format::FormatAction, nav::NavAction, structure::StructAction,
};
#[cfg(test)]
use crate::model::ArrowKey;
use crate::model::{
    SafeFontFamily,
    style_types::{BorderSide, BorderWeight, HexColor},
};

/// Top-level action dispatched from a keyboard event.
///
/// [`crate::input::keyboard::classify_key`] maps a key + modifier combination to one
/// of these variants. [`crate::input::keyboard::execute`] routes each variant to its
/// category module (`nav`, `edit`, `format`, `structure`). `Copy`, `Cut`, and
/// `Paste` are handled inline in `Workbook` because they need `AppClipboard`
/// and async OS clipboard APIs.
#[derive(Debug, Clone, PartialEq)]
pub enum SpreadsheetAction {
    Nav(NavAction),
    Edit(EditAction),
    Format(FormatAction),
    Structure(StructAction),
    /// Clipboard actions are handled by the Workbook component directly
    /// (they need the AppClipboard store and async OS clipboard APIs).
    Copy,
    Cut,
    Paste,
}

// Convenience constructors
// Used by the toolbar and other components to avoid deep nesting like
// `SpreadsheetAction::Format(FormatAction::ToggleBold)`.
impl SpreadsheetAction {
    #[cfg(test)]
    pub fn navigate(dir: ArrowKey) -> Self {
        Self::Nav(NavAction::Arrow(dir))
    }
    #[cfg(test)]
    pub fn start_edit(text: String) -> Self {
        Self::Edit(EditAction::Start(text))
    }
    #[cfg(test)]
    pub fn commit(dir: ArrowKey) -> Self {
        Self::Edit(EditAction::CommitAndNavigate(dir))
    }
    pub fn toggle_bold() -> Self {
        Self::Format(FormatAction::ToggleBold)
    }
    pub fn toggle_italic() -> Self {
        Self::Format(FormatAction::ToggleItalic)
    }
    pub fn toggle_underline() -> Self {
        Self::Format(FormatAction::ToggleUnderline)
    }
    pub fn toggle_strikethrough() -> Self {
        Self::Format(FormatAction::ToggleStrikethrough)
    }
    pub fn set_font_size(size: f64) -> Self {
        Self::Format(FormatAction::SetFontSize(size))
    }
    pub fn set_font_family(family: SafeFontFamily) -> Self {
        Self::Format(FormatAction::SetFontFamily(family))
    }
    pub fn set_text_color(hex: HexColor) -> Self {
        Self::Format(FormatAction::SetTextColor(hex))
    }
    pub fn set_background_color(hex: HexColor) -> Self {
        Self::Format(FormatAction::SetBackgroundColor(hex))
    }
    pub fn set_border(side: BorderSide, weight: BorderWeight, color: HexColor) -> Self {
        Self::Format(FormatAction::SetBorder {
            side,
            weight,
            color,
        })
    }
    pub fn set_num_fmt(code: &str) -> Self {
        Self::Format(FormatAction::SetNumFmt(code.to_owned()))
    }
    pub fn clear_formatting() -> Self {
        Self::Format(FormatAction::ClearFormatting)
    }
    pub fn set_h_align(align: HorizontalAlignment) -> Self {
        Self::Format(FormatAction::SetHorizontalAlign(align))
    }
    pub fn set_v_align(align: VerticalAlignment) -> Self {
        Self::Format(FormatAction::SetVerticalAlign(align))
    }
    pub fn increase_decimals() -> Self {
        Self::Format(FormatAction::IncreaseDecimals)
    }
    pub fn decrease_decimals() -> Self {
        Self::Format(FormatAction::DecreaseDecimals)
    }
    pub fn undo() -> Self {
        Self::Structure(StructAction::Undo)
    }
    pub fn redo() -> Self {
        Self::Structure(StructAction::Redo)
    }
}
