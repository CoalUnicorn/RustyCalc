//! "Cond. Format" toolbar button — opens the Conditional Formatting modal.

use leptos::prelude::*;

use crate::state::WorkbookState;

#[component]
pub fn ConditionalFormattingButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let on_click = move |_: web_sys::MouseEvent| {
        state.cf_dialog_open.set(true);
    };
    view! {
        <button class="tb-btn" title="Conditional formatting" on:click=on_click>
            "Cond. Format"
        </button>
    }
}
