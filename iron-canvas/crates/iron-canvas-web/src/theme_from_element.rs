//! CSS-var bridge for `CanvasTheme`. Wasm-only — `CanvasTheme` itself lives
//! in `iron-canvas-core`, but the `web_sys::Element` + `getComputedStyle`
//! plumbing stays here.

use crate::theme::{CanvasTheme, ThemeVariables};

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

/// Build a theme from CSS custom properties on `document.documentElement`.
/// Mirrors IronCalc upstream's default theme target. Falls back to
/// `CanvasTheme::light()` if the document or root element is missing.
pub fn from_root() -> CanvasTheme {
    let Some(window) = web_sys::window() else {
        return CanvasTheme::light();
    };
    let Some(doc) = window.document() else {
        return CanvasTheme::light();
    };
    let Some(el) = doc.document_element() else {
        return CanvasTheme::light();
    };
    from_element(&el)
}
