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

    let formula_msg = Memo::new(move |_| -> Option<String> {
        let edit = state.editing_cell.get()?;
        if !edit.text.starts_with('=') {
            return None;
        }
        let a = &edit.formula_analysis;
        if let Some(e) = &a.parse_error {
            return Some(format!("Parse error at col {}: {}", e.position, e.message));
        }
        if a.validation_error.is_some() {
            return Some("Syntax error in formula".into());
        }
        let n = a.invalid_refs.len() + a.invalid_functions.len();
        if n > 0 {
            return Some(format!("{n} unresolved name(s)"));
        }
        None
    });

    view! {
        <div class="status-bar">
            {move || match state.status.get() {
                None => view! { <span /> }.into_any(),
                Some(StatusMessage::Error(msg)) => {
                    view! { <span class="status-bar-error">{msg}</span> }.into_any()
                }
            }}
            {move || match formula_msg.get() {
                None => view! { <span /> }.into_any(),
                Some(msg) => {
                    view! { <span class="status-bar-formula-error">{msg}</span> }.into_any()
                }
            }}
        </div>
    }
}
