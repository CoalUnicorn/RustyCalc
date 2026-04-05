use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};

// CssColor

/// A CSS hex color string, e.g. `"#FF0000"`. Never empty.
/// The inner field is private; construct via `CssColor::new`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CssColor(String);

impl CssColor {
    /// Constructs a `CssColor`. Substitutes `"#000000"` for empty inputs.
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() {
            Self("#000000".to_owned())
        } else {
            Self(s)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// SafeFontFamily

/// Font families the browser can reliably render.
/// Unknown font names from Excel files map to `SystemUi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SafeFontFamily {
    Arial,
    /// Renders as `"Calibri, system-ui"`. On Linux/Android, `system-ui` activates -
    /// accepted approximation.
    CalibriLike,
    CourierNew,
    Georgia,
    TimesNewRoman,
    Verdana,
    /// Fallback for any unrecognised font name.
    SystemUi,
}

/// Font name variants for different contexts.
#[derive(Debug, Clone, Copy)]
struct FontNames {
    css: &'static str,
    model: &'static str,
    label: &'static str,
}

impl SafeFontFamily {
    /// Get all name variants for this font family.
    fn names(&self) -> FontNames {
        match self {
            Self::Arial => FontNames {
                css: "Arial",
                model: "Arial",
                label: "Arial",
            },
            Self::CalibriLike => FontNames {
                css: "Calibri, system-ui",
                model: "Calibri",
                label: "Calibri",
            },
            Self::CourierNew => FontNames {
                css: "Courier New",
                model: "Courier New",
                label: "Courier New",
            },
            Self::Georgia => FontNames {
                css: "Georgia",
                model: "Georgia",
                label: "Georgia",
            },
            Self::TimesNewRoman => FontNames {
                css: "Times New Roman",
                model: "Times New Roman",
                label: "Times New Roman",
            },
            Self::Verdana => FontNames {
                css: "Verdana",
                model: "Verdana",
                label: "Verdana",
            },
            Self::SystemUi => FontNames {
                css: "system-ui",
                model: "Arial",
                label: "System",
            },
        }
    }

    /// CSS `font-family` value (may include fallback).
    pub fn css_name(&self) -> &'static str {
        self.names().css
    }

    /// The name stored in IronCalc's `Style.font.name`.
    pub fn model_name(&self) -> &'static str {
        self.names().model
    }

    /// Display label for the toolbar.
    pub fn label(&self) -> &'static str {
        self.names().label
    }

    /// All font families in menu order.
    pub const ALL: &[SafeFontFamily] = &[
        Self::Arial,
        Self::CalibriLike,
        Self::CourierNew,
        Self::Georgia,
        Self::TimesNewRoman,
        Self::Verdana,
    ];
}

impl From<Option<&str>> for SafeFontFamily {
    fn from(name: Option<&str>) -> Self {
        match name {
            Some("Arial") => Self::Arial,
            Some("Calibri") => Self::CalibriLike,
            Some("Courier New") => Self::CourierNew,
            Some("Georgia") => Self::Georgia,
            Some("Times New Roman") => Self::TimesNewRoman,
            Some("Verdana") => Self::Verdana,
            _ => Self::SystemUi,
        }
    }
}

// ResolvedFont
#[derive(Debug, Clone)]
pub struct ResolvedFont {
    pub size_px: f64,
    // pub bold: bool,
    // pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    // pub family: SafeFontFamily,
    /// Pre-built canvas `ctx.set_font()` string, e.g. `"bold italic 12px Arial"`.
    pub css: String,
}

impl ResolvedFont {
    pub(crate) fn build(size_px: f64, bold: bool, italic: bool, family: &SafeFontFamily) -> String {
        let b = if bold { "bold " } else { "" };
        let i = if italic { "italic " } else { "" };
        format!("{b}{i}{size_px}px {}", family.css_name())
    }
}

/// Basic formatting
#[derive(Debug, Clone, PartialEq)]
pub struct TextFormat {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

/// Visual style properties.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: f64,
    pub font_family: SafeFontFamily,
    pub h_align: HorizontalAlignment,
    pub text_color: CssColor,
    pub bg_color: Option<CssColor>,
}
// ResolvedCellStyle

/// Everything the renderer needs to paint one cell. No further resolution required.
// IMPORTANT: this is style we getting from ironcalc_base /home/mmm/01_Dev/IronCalc/base/src/types.rs
// Our FrontendModel fn cell_style() returns this. also see /home/mmm/01_Dev/IronCalc/base/src/user_model
#[derive(Debug, Clone)]
pub struct ResolvedCellStyle {
    /// Resolved text color; never empty.
    pub text_color: CssColor,
    /// `None` = transparent (skip the fillRect call).
    // pub bg_color: Option<CssColor>,
    pub font: ResolvedFont,
    /// `General` already resolved to `Left` or `Right` based on cell type.
    pub h_align: HorizontalAlignment,
    pub v_align: VerticalAlignment,
    pub wrap_text: bool,
}

// ToolbarState

#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarState {
    pub format: TextFormat,
    pub style: TextStyle,
}

// Sheet dimension

/// The used data extent of the active sheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SheetDimension {
    pub min_row: i32,
    pub min_column: i32,
    pub max_row: i32,
    pub max_column: i32,
}

// Direction enums

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrowKey {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageDir {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

use ironcalc_base::UserModel;

use crate::state::EditingCell;
impl CellAddress {
    pub fn from_view(model: &UserModel<'static>) -> Self {
        let m = model.get_selected_view();
        Self {
            sheet: m.sheet,
            row: m.row,
            column: m.column,
        }
    }

    pub fn from_editing(cell: &EditingCell) -> Self {
        Self {
            sheet: cell.address.sheet,
            row: cell.address.row,
            column: cell.address.column,
        }
    }
}

// Frozen pane state

/// Number of frozen rows and columns on the active sheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrozenPanes {
    pub rows: i32,
    pub cols: i32,
}

impl FrozenPanes {
    /// True if any rows or columns are frozen.
    pub fn is_frozen(&self) -> bool {
        self.rows > 0 || self.cols > 0
    }
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_color_new_non_empty() {
        let c = CssColor::new("#FF0000");
        assert_eq!(c.as_str(), "#FF0000");
    }

    #[test]
    fn css_color_new_empty_substitutes_black() {
        let c = CssColor::new("");
        assert_eq!(c.as_str(), "#000000");
    }

    #[test]
    fn safe_font_family_known_names() {
        assert_eq!(SafeFontFamily::from(Some("Arial")), SafeFontFamily::Arial);
        assert_eq!(
            SafeFontFamily::from(Some("Calibri")),
            SafeFontFamily::CalibriLike
        );
        assert_eq!(
            SafeFontFamily::from(Some("Courier New")),
            SafeFontFamily::CourierNew
        );
        assert_eq!(
            SafeFontFamily::from(Some("Times New Roman")),
            SafeFontFamily::TimesNewRoman
        );
    }

    #[test]
    fn safe_font_family_unknown_falls_back() {
        assert_eq!(
            SafeFontFamily::from(Some("Wingdings")),
            SafeFontFamily::SystemUi
        );
        assert_eq!(SafeFontFamily::from(None), SafeFontFamily::SystemUi);
    }

    #[test]
    fn safe_font_family_css_names() {
        assert_eq!(SafeFontFamily::Arial.css_name(), "Arial");
        assert_eq!(SafeFontFamily::CourierNew.css_name(), "Courier New");
        assert_eq!(SafeFontFamily::SystemUi.css_name(), "system-ui");
    }

    #[test]
    fn resolved_font_build_bold_italic() {
        let css = ResolvedFont::build(12.0, true, true, &SafeFontFamily::Arial);
        assert_eq!(css, "bold italic 12px Arial");
    }

    #[test]
    fn resolved_font_build_plain() {
        let css = ResolvedFont::build(11.0, false, false, &SafeFontFamily::CalibriLike);
        assert_eq!(css, "11px Calibri, system-ui");
    }
}
