use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};

use crate::model::style_types::BorderWeight;

/// CSS hex color string, e.g. `"#FF0000"`. Empty input becomes `"#000000"`.
/// CssColor is a wire type — it carries model data zero-copy through the renderer
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CssColor(String);

impl CssColor {
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() {
            Self("#000000".to_owned())
        } else {
            Self(s.to_lowercase())
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Font families the browser can reliably render. Unknown names map to `SystemUi`.
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
    /// Fallback for any unrecognized font name.
    SystemUi,
}

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

    pub fn css_name(&self) -> &'static str {
        self.names().css
    }

    pub fn model_name(&self) -> &'static str {
        self.names().model
    }

    pub fn label(&self) -> &'static str {
        self.names().label
    }

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

#[derive(Debug, Clone, PartialEq)]
pub struct TextFormat {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font_size: f64,
    pub font_family: SafeFontFamily,
    pub h_align: HorizontalAlignment,
    pub v_align: VerticalAlignment,
    pub text_color: CssColor,
    pub bg_color: Option<CssColor>,
}

/// Active cell border state reflected in the toolbar's border picker.
///
/// Holds the *picker's* current color/weight choices — what a click on a preset
/// will apply — seeded from the active cell's dominant border when the dropdown
/// opens (see `SheetQuery::toolbar_state`).
#[derive(Debug, Clone, PartialEq)]
pub struct BorderState {
    pub color: CssColor,
    pub weight: BorderWeight,
}

impl Default for BorderState {
    fn default() -> Self {
        Self {
            color: CssColor::new("#000000"),
            weight: BorderWeight::Thin,
        }
    }
}

/// Active cell style state reflected in the toolbar.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolbarState {
    pub format: TextFormat,
    pub style: TextStyle,
    pub border: BorderState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrowKey {
    Up,
    Down,
    Left,
    Right,
}

impl ArrowKey {
    /// (row-delta, column-delta) for an arrow key.
    pub fn delta(&self) -> (i32, i32) {
        match self {
            ArrowKey::Down => (1, 0),
            ArrowKey::Up => (-1, 0),
            ArrowKey::Left => (0, -1),
            ArrowKey::Right => (0, 1),
        }
    }

    pub fn from_str(key: &str) -> Option<Self> {
        match key {
            "ArrowDown" => Some(ArrowKey::Down),
            "ArrowUp" => Some(ArrowKey::Up),
            "ArrowLeft" => Some(ArrowKey::Left),
            "ArrowRight" => Some(ArrowKey::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageDir {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrozenPanes {
    pub rows: i32,
    pub cols: i32,
}

impl FrozenPanes {
    pub fn is_frozen(&self) -> bool {
        self.rows > 0 || self.cols > 0
    }
}
