use crate::components::workbook::Workbook;

use leptos::prelude::*;
use leptos_use::use_interval_fn;

use crate::state::WorkbookState;
use crate::storage;

#[component]
pub fn App() -> impl IntoView {
    // Load the previously selected workbook from localStorage, or create a
    // fresh blank one if localStorage is empty (first launch).
    let (uuid, model) = storage::load_selected().unwrap_or_else(storage::create_new);

    let wb_state = WorkbookState::new();
    wb_state.current_uuid.set(Some(uuid));

    let model = StoredValue::new_local(model);

    // Internal clipboard — mirrors what was last copied/cut, so Ctrl+V can
    // paste even if the OS clipboard is unavailable (sandboxed iframe, etc.).
    let clipboard: StoredValue<Option<crate::model::AppClipboard>, LocalStorage> =
        StoredValue::new_local(None);

    provide_context(wb_state);
    provide_context(model);
    provide_context(clipboard);

    // Auto-save interval
    // Every second, flush the model's pending diff queue. If there are unsaved
    // mutations, persist to localStorage. Cleanup is automatic on unmount.
    use_interval_fn(
        move || {
            let Some(uuid) = wb_state.current_uuid.get_untracked() else {
                return;
            };
            let mut has_changes = false;
            model.update_value(|m| {
                has_changes = !m.flush_send_queue().is_empty();
            });
            if has_changes {
                model.with_value(|m| storage::save(&uuid, m));
            }
        },
        1000,
    );

    // Row layout: collapsible drawer on the left, workbook editor fills the rest.

    view! {
        <div id="app">
            <Workbook />
        </div>
    }
}
