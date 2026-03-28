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

    let on_toggle_theme = move |_: web_sys::MouseEvent| {
        let new_theme = state.theme.get().toggle();
        state.theme.set(new_theme);
        new_theme.save();
        apply_theme_to_dom(new_theme);
        state.request_redraw();
    };

    let on_toggle_perf = move |_: web_sys::MouseEvent| {
        state.show_perf_panel.update(|v| *v = !*v);
    };

    // Show icon for the CURRENT theme (☀ = light mode active, ☾ = dark mode active).
    let theme_icon = move || {
        if state.theme.get() == Theme::Dark {
            "☀ Light"
        } else {
            "☾ Dark"
        }
    };

    let perf_icon = move || {
        if state.show_perf_panel.get() {
            "⏱ Hide Perf"
        } else {
            "⏱ Show Perf"
        }
    };

    view! {
        <div class="file-bar">
            <button class="file-bar-btn" on:click=on_toggle_theme title="Toggle theme">
                {theme_icon}
            </button>
            <button class="file-bar-btn" on:click=on_toggle_perf title="Toggle performance panel">
                {perf_icon}
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
