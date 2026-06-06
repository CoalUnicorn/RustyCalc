//! "Cond. Format" toolbar button — opens the Conditional Formatting modal.

use leptos::prelude::*;

use crate::state::WorkbookState;

#[component]
pub fn ConditionalFormattingButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    // Mutual exclusion: only one drawer is open at a time, and any in-progress
    // range pick is cancelled when switching surfaces.
    let on_click = move |_: web_sys::MouseEvent| {
        state.named_ranges_modal_open.set(false);
        state.range_capture.set(None);
        state.cf_dialog_open.set(true);
    };
    view! {
        <button class="tb-btn" title="Conditional formatting" on:click=on_click>
            "Cond. Format"
        </button>
    }
}
