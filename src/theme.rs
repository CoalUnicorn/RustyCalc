/// Enhanced Theme system for IronCalc Leptos with leptos-use integration.
///
/// Three parallel mechanisms:
/// - leptos-use `use_color_mode()` handles system theme detection, localStorage persistence, and DOM class management
/// - [`Theme`] provides compatibility layer and extends leptos-use ColorMode
/// - [`CanvasTheme`] carries concrete color strings for the Canvas 2D API, which cannot consume CSS variables
use gloo_storage::{LocalStorage, Storage};
use leptos_use::{use_color_mode_with_options, UseColorModeOptions, ColorMode};
use leptos::prelude::*;

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

// Enhanced Theme enum that extends leptos-use ColorMode

/// Theme enum that works with both leptos-use ColorMode and our canvas theming
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    /// Automatically detect from system preference (light/dark)
    Auto,
    /// Force light theme
    Light,
    /// Force dark theme
    Dark,
}

/// Convert between our Theme and leptos-use ColorMode
impl From<ColorMode> for Theme {
    fn from(mode: ColorMode) -> Self {
        match mode {
            ColorMode::Auto => Theme::Auto,
            ColorMode::Light => Theme::Light,
            ColorMode::Dark => Theme::Dark,
            ColorMode::Custom(_) => Theme::Light, // Fallback for custom modes
        }
    }
}

impl From<Theme> for ColorMode {
    fn from(theme: Theme) -> Self {
        match theme {
            Theme::Auto => ColorMode::Auto,
            Theme::Light => ColorMode::Light,
            Theme::Dark => ColorMode::Dark,
        }
    }
}

/// Initialize leptos-use color mode with RustyCalc-specific settings
pub fn use_rusty_calc_theme() -> leptos_use::UseColorModeReturn {
    use_color_mode_with_options(
        UseColorModeOptions::default()
            .storage_key("ironcalc_theme") // Use existing storage key for migration
            .initial_value(ColorMode::Auto) // Default to auto-detection
            .attribute("class") // Add light/dark classes to <html>
            .emit_auto(false) // Never emit Auto in the mode signal, always resolve to Light/Dark
    )
}

impl Theme {
    pub const STORAGE_KEY: &'static str = "ironcalc_theme";

    /// Retrieve the last saved preference from localStorage with Auto support.
    /// Enhanced to detect system theme when Auto is selected.
    /// Falls back to `Light` if nothing is stored.
    pub fn from_storage() -> Self {
        let s: String = LocalStorage::get(Self::STORAGE_KEY).unwrap_or_default();
        match s.as_str() {
            "auto" => Theme::Auto,
            "dark" => Theme::Dark,
            _ => Theme::Light,
        }
    }

    /// Get system preference (true if system prefers dark mode)
    /// Uses CSS media query to detect system preference
    pub fn system_prefers_dark() -> bool {
        let window = web_sys::window().unwrap();
        if let Ok(media_query) = window.match_media("(prefers-color-scheme: dark)") {
            media_query.unwrap().matches()
        } else {
            false // Fallback to light if media query fails
        }
    }

    /// Persist the current preference to localStorage.
    /// DEPRECATED: leptos-use handles storage automatically
    pub fn save(self) {
        LocalStorage::set(Self::STORAGE_KEY, self.as_str()).ok();
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// Toggle between Light and Dark (preserves Auto mode)
    pub fn toggle(self) -> Self {
        match self {
            Theme::Auto => Theme::Auto, // Keep auto mode
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }

    /// Resolve theme to actual Light/Dark value, considering system preference
    /// This is needed for canvas theming since Canvas 2D API needs concrete colors
    pub fn resolve_actual_theme(self, system_prefers_dark: bool) -> Theme {
        match self {
            Theme::Auto => if system_prefers_dark { Theme::Dark } else { Theme::Light },
            Theme::Light => Theme::Light,
            Theme::Dark => Theme::Dark,
        }
    }

    /// Resolve theme using current system preference (convenience method)
    pub fn resolve_with_system(self) -> Theme {
        self.resolve_actual_theme(Self::system_prefers_dark())
    }

    /// Return the canvas color palette for this theme.
    /// For Auto themes, pass the resolved theme from system preference.
    pub fn canvas_theme(self) -> &'static CanvasTheme {
        match self {
            Theme::Auto => &LIGHT, // Fallback, should use resolve_actual_theme() first
            Theme::Light => &LIGHT,
            Theme::Dark => &DARK,
        }
    }

    /// Get canvas theme resolved from system preference
    pub fn canvas_theme_resolved(self, system_prefers_dark: bool) -> &'static CanvasTheme {
        self.resolve_actual_theme(system_prefers_dark).canvas_theme()
    }
}

// CanvasTheme

/// Concrete color strings for the Canvas 2D rendering context.
/// One static instance per theme; passed into `CanvasRenderer::new()`.
#[derive(Copy, Clone, PartialEq)]
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
