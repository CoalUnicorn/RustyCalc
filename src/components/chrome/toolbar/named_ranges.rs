//! "Names" toolbar button — opens the Manage Named Ranges modal.

use leptos::prelude::*;

use crate::state::WorkbookState;

#[component]
pub fn NamedRangesButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let on_click = move |_: web_sys::MouseEvent| {
        state.named_ranges_modal_open.set(true);
    };
    view! {
        <button
            class="tb-btn"
            title="Manage named ranges"
            on:click=on_click
        >
            "Names"
        </button>
    }
}
