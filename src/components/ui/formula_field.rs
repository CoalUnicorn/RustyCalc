//! Reusable formula text field — a `<textarea>` with a colored ref-token
//! overlay behind it and a validation class on the host.
//!
//! This is the drawer/form counterpart to the in-grid
//! [`crate::components::workbook::editing::formula_text_area::FormulaTextArea`]
//! and the [`crate::components::chrome::formula_bar::FormulaBar`]: it shares
//! their [`FormulaOverlay`] + `.fe-*` alignment machinery, but carries none of
//! the cell-editing coupling (no `editing_cell`, no canvas positioning, no
//! point-mode). It is a pure view + input adapter.
//!
//! ## Storage-agnostic by design
//!
//! The field does **not** own its text. The caller passes `value` (a read
//! signal) and an `on_input` callback that receives `(value, cursor)` on every
//! keystroke. This lets two very different panels share one component:
//!
//! - Conditional Formatting drives a local `RwSignal<String>` + a local
//!   `analyze_formula` Memo for `refs` / `is_error`.
//! - Manage Named Ranges keeps its `editing_named_range` + `sync_edit`
//!   pipeline and supplies derived read signals + a `sync_edit` write callback.
//!
//! `refs` and `is_error` are likewise caller-derived from whichever
//! [`crate::input::formula::FormulaAnalysis`] the panel already computes — the
//! component never runs the analyzer itself.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::workbook::editing::formula_overlay::FormulaOverlay;
use crate::coord::ActiveRef;
use crate::input::formula::{read_value_and_cursor, suppress_navigation_defaults};

#[component]
pub fn FormulaField(
    /// Current formula text (read-only here; the caller owns the storage).
    #[prop(into)]
    value: Signal<String>,
    /// Colored ref tokens to paint behind the text, from the caller's analysis.
    #[prop(into)]
    refs: Signal<Vec<ActiveRef>>,
    /// Drives the host's `error` class (red border) — caller's validity verdict.
    #[prop(into)]
    is_error: Signal<bool>,
    /// Called on every keystroke with `(value, cursor)`. The caller persists
    /// both (cursor matters for future point-mode ref splicing).
    on_input: Callback<(String, usize)>,
    /// Visible rows of the textarea (default 1).
    #[prop(optional)]
    rows: Option<u32>,
    #[prop(optional, into)] placeholder: String,
) -> impl IntoView {
    // Cache the overlay element so the scroll handler doesn't query the DOM
    // every tick — same pattern as FormulaBar.
    let overlay_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    let handle_input = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some((v, cursor)) = read_value_and_cursor(&target) else {
            return;
        };
        on_input.run((v, cursor));
    };

    // Enter/Tab/Escape: swallow the browser default but let the event bubble
    // so the drawer's own buttons / Esc handler stay in control.
    let on_keydown = move |ev: web_sys::KeyboardEvent| suppress_navigation_defaults(&ev);

    // Keep the overlay glued to the textarea's scroll position so long
    // formulas stay aligned.
    let on_scroll = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else { return };
        let Some(ta) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() else {
            return;
        };
        if let Some(overlay) = overlay_ref.get() {
            overlay.set_scroll_top(ta.scroll_top());
            overlay.set_scroll_left(ta.scroll_left());
        }
    };

    let host_class = move || {
        if is_error.get() {
            "fe-host ff-host error"
        } else {
            "fe-host ff-host"
        }
    };

    view! {
        <div class=host_class>
            <FormulaOverlay node_ref=overlay_ref text=value refs=refs multiline=true />
            <textarea
                class="ff-input fe-text"
                rows=rows.unwrap_or(1)
                placeholder=placeholder
                prop:value=move || value.get()
                on:input=handle_input
                on:keydown=on_keydown
                on:scroll=on_scroll
            />
        </div>
    }
}
