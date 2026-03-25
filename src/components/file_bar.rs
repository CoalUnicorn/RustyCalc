use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::WorkbookState;
use crate::theme::Theme;

#[component]
pub fn FileBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Apply the initial theme to <html> on mount, so CSS variables match
    // the stored preference from the start.
    Effect::new(move |_| {
        apply_theme_to_dom(state.theme.get());
    });

    let on_toggle = move |_: web_sys::MouseEvent| {
        let new_theme = state.theme.get().toggle();
        state.theme.set(new_theme);
        new_theme.save();
        apply_theme_to_dom(new_theme);
        state.request_redraw();
    };

    let btn = "display:flex;align-items:center;justify-content:center;\
               min-height:44px;padding:0 10px;font-size:12px;\
               border:1px solid var(--border-color);border-radius:8px;\
               cursor:pointer;background:var(--btn-bg);color:var(--text-primary);";

    // Show icon for the CURRENT theme (☀ = light mode active, ☾ = dark mode active).
    let theme_icon = move || {
        if state.theme.get() == Theme::Dark {
            "☀ Light"
        } else {
            "☾ Dark"
        }
    };

    view! {
        <div style="display:flex;align-items:center;height:44px;\
            padding:0 8px;background:var(--bg-secondary);\
            border-bottom:1px solid var(--border-color);flex-shrink:0;">
            <button style=btn on:click=on_toggle title="Toggle theme">
                {theme_icon}
            </button>
        </div>
    }
}

/// Set `data-theme` on `<html>` so CSS variables switch.
fn apply_theme_to_dom(theme: Theme) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(el) = doc.document_element() {
            el.unchecked_ref::<web_sys::HtmlElement>()
                .dataset()
                .set("theme", theme.as_str())
                .ok();
        }
    }
}
