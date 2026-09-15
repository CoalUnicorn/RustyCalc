//! CSS-var bridge for `CanvasTheme`. Wasm-only — `CanvasTheme` itself lives
//! in `iron-canvas-core`, but the `web_sys::Element` + `getComputedStyle`
//! plumbing stays here.

use iron_canvas_core::theme::{CanvasTheme, ThemeVariables};

/// Build a theme from CSS custom properties on `el`'s computed style.
/// Reads the upstream `--palette-*` keys via `getComputedStyle` and
/// pipes them through `ThemeVariables::from_css_reader`. Silently falls
/// back to `CanvasTheme::light()` when `window`/`getComputedStyle` are
/// absent (SSR, detached node) — the renderer always has a usable
/// palette, the bridge never panics.
pub fn from_element(el: &web_sys::Element) -> CanvasTheme {
    let Some(window) = web_sys::window() else {
        return CanvasTheme::light();
    };
    let Ok(Some(style)) = window.get_computed_style(el) else {
        return CanvasTheme::light();
    };
    ThemeVariables::from_css_reader(|key| style.get_property_value(key).ok()).build()
}
