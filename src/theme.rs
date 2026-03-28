/// Theme system for IronCalc Leptos.
///
/// Two parallel mechanisms:
/// - [`Theme`] + CSS variables (index.html) handle HTML component colors.
/// - [`CanvasTheme`] carries concrete color strings for the Canvas 2D API,
///   which cannot consume CSS variables.
use gloo_storage::{LocalStorage, Storage};

// Shared color palette
// TODO: create a component
/// 40-color palette used by the tab color picker and future color pickers.
pub const COLOR_PALETTE: &[&str] = &[
    "#000000", "#FFFFFF", "#FF0000", "#FF4500", "#FF8C00", "#FFD700", "#00CC44", "#008000",
    "#00BFFF", "#0000FF", "#C00000", "#FF6666", "#FF9966", "#FFCC44", "#AADD44", "#44AA66",
    "#44BBCC", "#4477DD", "#7755BB", "#CC44CC", "#7F0000", "#CC3333", "#CC6633", "#CC9922",
    "#88BB22", "#228844", "#228899", "#224499", "#553388", "#882288", "#400000", "#800000",
    "#804000", "#808000", "#406000", "#004000", "#004040", "#000080", "#400080", "#800040",
];

// Theme enum

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub const STORAGE_KEY: &'static str = "ironcalc_theme";

    /// Retrieve the last saved preference from localStorage.
    /// Falls back to `Light` if nothing is stored.
    pub fn from_storage() -> Self {
        let s: String = LocalStorage::get(Self::STORAGE_KEY).unwrap_or_default();
        if s == "dark" {
            Theme::Dark
        } else {
            Theme::Light
        }
    }

    /// Persist the current preference to localStorage.
    pub fn save(self) {
        LocalStorage::set(Self::STORAGE_KEY, self.as_str()).ok();
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }

    /// Return the canvas color palette for this theme.
    pub fn canvas_theme(self) -> &'static CanvasTheme {
        match self {
            Theme::Light => &LIGHT,
            Theme::Dark => &DARK,
        }
    }
}

// CanvasTheme

/// Concrete color strings for the Canvas 2D rendering context.
/// One static instance per theme; passed into `CanvasRenderer::new()`.
pub struct CanvasTheme {
    pub grid_color: &'static str,
    pub grid_separator_color: &'static str,
    pub header_bg: &'static str,
    pub header_border_color: &'static str,
    pub header_text_color: &'static str,
    pub header_selected_bg: &'static str,
    pub header_selected_color: &'static str,
    pub default_text_color: &'static str,
    pub selection_color: &'static str,
    pub cell_bg: &'static str,
    /// rgba() string for the semi-transparent range selection fill.
    pub selection_fill: &'static str,
}

pub static LIGHT: CanvasTheme = CanvasTheme {
    grid_color: "#E0E0E0",
    grid_separator_color: "#E0E0E0",
    header_bg: "#FFF",
    header_border_color: "#E0E0E0",
    header_text_color: "#333",
    header_selected_bg: "#EEEEEE",
    header_selected_color: "#333",
    default_text_color: "#2E414D",
    selection_color: "#17A2D3",
    cell_bg: "#FFFFFF",
    selection_fill: "rgba(23,162,211,0.12)",
};

pub static DARK: CanvasTheme = CanvasTheme {
    grid_color: "#3A3A3A",
    grid_separator_color: "#3A3A3A",
    header_bg: "#1E1E1E",
    header_border_color: "#3A3A3A",
    header_text_color: "#CCC",
    header_selected_bg: "#2D2D2D",
    header_selected_color: "#CCC",
    default_text_color: "#D4D4D4",
    selection_color: "#17A2D3",
    cell_bg: "#121212",
    selection_fill: "rgba(23,162,211,0.18)",
};
