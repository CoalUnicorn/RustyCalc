/// Theme system for RustyCalc.
///
/// - leptos-use `use_color_mode()` handles system theme detection,
///   localStorage persistence, and writes `data-theme="light|dark"` on
///   `<html>`.
/// - [`Theme`] enum mirrors leptos-use `ColorMode` (Auto/Light/Dark) for
///   the consumer-side UI bits that need a strongly-typed handle.
/// - The canvas palette is no longer plumbed through this module: iron-canvas
///   reads its theme directly from CSS custom properties on `<html>` via
///   `IronCanvas::set_theme_from_element`. `CanvasTheme` and
///   [`ThemeVariables`] are re-exported here only for callers that want to
///   build a theme programmatically (e.g. tests).
use leptos_use::{use_color_mode_with_options, ColorMode, UseColorModeOptions};

#[allow(unused_imports)]
pub use iron_canvas::theme::{CanvasTheme, ThemeVariables};

// Shared color palette
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

/// Initialize leptos-use color mode with RustyCalc-specific settings.
///
/// Sets `data-theme` on `<html>` to "light" or "dark", matching the CSS
/// selectors used throughout the app. Persists the preference in localStorage
/// under `ironcalc_theme`. Auto resolves to the OS preference.
pub fn use_rusty_calc_theme() -> leptos_use::UseColorModeReturn {
    use_color_mode_with_options(
        UseColorModeOptions::default()
            .storage_key("ironcalc_theme")
            .initial_value(ColorMode::Auto)
            .attribute("data-theme") // Sets <html data-theme="dark|light">
            .emit_auto(false), // Always resolve Auto to a concrete mode
    )
}

