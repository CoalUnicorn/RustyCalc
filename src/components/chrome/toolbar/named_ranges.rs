//! "Names" toolbar button — opens the Manage Named Ranges modal.

use leptos::prelude::*;

use super::icon::{Icon, SheetIcon};
use crate::state::WorkbookState;

#[component]
pub fn NamedRangesButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    // Mutual exclusion: only one drawer is open at a time, and any in-progress
    // range pick is cancelled when switching surfaces.
    let on_click = move |_: web_sys::MouseEvent| {
        state.cf_dialog_open.set(false);
        state.range_capture.set(None);
        state.named_ranges_modal_open.set(true);
    };
    view! {
        <button
            class="tb-btn"
            title="Manage named ranges"
            on:click=on_click
        >
            <Icon icon=SheetIcon::NamedRange /> "Names"
        </button>
    }
}
