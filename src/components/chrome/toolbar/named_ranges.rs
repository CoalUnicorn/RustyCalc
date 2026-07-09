//! "Names" toolbar button — opens the Manage Named Ranges modal.

use leptos::prelude::*;

use super::icon::{Icon, SheetIcon};
use crate::state::ActiveDrawer;
use crate::state::WorkbookState;

#[component]
pub fn NamedRangesButton() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let on_click = move |_: web_sys::MouseEvent| {
        state.range_capture.set(None);
        state.active_drawer.set(Some(ActiveDrawer::NamedRanges));
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
