//! JSON view: the selected attempt's exported JSON.
//!
//! The panel builds the JSON lazily — only while this view is open — and this
//! view renders the string it is given. Copy and export report through
//! callbacks; the panel publishes the commands, which one worksheet
//! coordinator drains.

use leptos::prelude::*;

/// Everything the JSON view renders.
#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct JsonView {
    pub text: String,
    pub has_attempt: bool,
}

/// Raw JSON for the selected attempt, with its copy and export actions.
#[component]
pub(super) fn InspectorJson(
    view: Memo<JsonView>,
    on_copy: Callback<()>,
    on_export: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="pp-json">
            <div class="pp-json-actions">
                <button
                    class="pp-action-btn"
                    type="button"
                    disabled=move || !view.get().has_attempt
                    title="Copy the selected attempt JSON"
                    on:click=move |_| on_copy.run(())
                >
                    "Copy attempt JSON"
                </button>
                <button
                    class="pp-action-btn"
                    type="button"
                    title="Download the selected capture as JSON"
                    on:click=move |_| on_export.run(())
                >
                    "Export capture JSON"
                </button>
            </div>
            {move || {
                let view = view.get();
                if view.text.is_empty() {
                    view! { <p class="pp-empty">"No attempt selected."</p> }.into_any()
                } else {
                    view! { <pre class="pp-json-text">{view.text}</pre> }.into_any()
                }
            }}
        </div>
    }
}
