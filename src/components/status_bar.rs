use leptos::prelude::*;

use crate::app_state::AppState;
use crate::components::perf_panel::PerfPanel;
use crate::input::formula_analysis::FormulaStatus;
use crate::state::{StatusMessage, WorkbookState};

/// Displays the most recent engine error below the sheet tab bar.
///
/// Clears automatically when the next action succeeds (`execute()` sets
/// `state.status` to `None` on `Ok`). Shows nothing when `state.status`
/// is `None`.
#[component]
pub fn StatusBar() -> impl IntoView {
    let app = expect_context::<AppState>();
    let state = expect_context::<WorkbookState>();

    let formula_msg = Memo::new(move |_| -> Option<String> {
        let edit = state.editing_cell.get()?;
        match &edit.formula_analysis.status {
            FormulaStatus::NotFormula | FormulaStatus::Valid { .. } => None,
            FormulaStatus::ParseError(e) => {
                Some(format!("Parse error at col {}: {}", e.position, e.message))
            }
            FormulaStatus::LexerError(e) => {
                Some(format!("Syntax error at col {}: {}", e.position, e.message))
            }
            FormulaStatus::Unresolved {
                refs,
                functions,
                names,
                ..
            } => Some(format!(
                "{} unresolved reference(s)",
                refs.len() + functions.len() + names.len()
            )),
        }
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
            <Show when=move || app.show_perf_panel.get()>
                <PerfPanel />
            </Show>
        </div>
    }
}
