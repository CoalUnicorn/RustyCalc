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
        apply_theme_to_dom(state.get_theme());
    });

    let on_toggle_theme = move |_: web_sys::MouseEvent| {
        state.toggle_theme(); // Use new toggle method with Auto support
        apply_theme_to_dom(state.get_theme());

        // Theme change event automatically fired by toggle_theme() → set_theme() → notify_theme_changed()
        web_sys::console::log_1(&format!("Theme changed to: {:?}", state.get_theme()).into());
    };

    let on_toggle_perf = move |_: web_sys::MouseEvent| {
        state.toggle_show_perf_panel();
    };

    // Show icon based on theme preference with Auto detection
    let theme_icon = move || {
        let preference = state.get_theme_preference();
        let resolved = state.get_theme();
        match preference {
            Theme::Auto => {
                if resolved == Theme::Dark {
                    "🌙 Auto (Dark)"
                } else {
                    "☀️ Auto (Light)"
                }
            }
            Theme::Light => "☾ Dark", // Currently light, click to go dark
            Theme::Dark => "☀ Light", // Currently dark, click to go light
        }
    };

    let perf_icon = move || {
        if state.get_show_perf_panel() {
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
