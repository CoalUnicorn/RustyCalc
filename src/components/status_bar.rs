use leptos::prelude::*;

use crate::state::{StatusMessage, WorkbookState};

/// Displays the most recent engine error below the sheet tab bar.
///
/// Clears automatically when the next action succeeds (`execute()` sets
/// `state.status` to `None` on `Ok`). Shows nothing when `state.status`
/// is `None`.
#[component]
pub fn StatusBar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    view! {
        <div class="status-bar">
            {move || match state.status.get() {
                None => view! { <span /> }.into_any(),
                Some(StatusMessage::Error(msg)) => {
                    view! { <span class="status-bar-error">{msg}</span> }.into_any()
                }
            }}
        </div>
    }
}
