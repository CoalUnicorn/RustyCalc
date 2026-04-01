use crate::components::workbook::Workbook;

use leptos::prelude::*;
use leptos_use::{use_debounce_fn, use_interval_fn};

use crate::state::WorkbookState;
use crate::{storage, storage_enhanced};

#[component]
pub fn App() -> impl IntoView {
    // Load the previously selected workbook from localStorage, or create a
    // fresh blank one if localStorage is empty (first launch).
    let (uuid, model) = storage::load_selected().unwrap_or_else(storage::create_new);

    let wb_state = WorkbookState::new();
    wb_state.set_current_uuid(Some(uuid));

    let model = StoredValue::new_local(model);

    // Internal clipboard — mirrors what was last copied/cut, so Ctrl+V can
    // paste even if the OS clipboard is unavailable (sandboxed iframe, etc.).
    let clipboard: StoredValue<Option<crate::model::AppClipboard>, LocalStorage> =
        StoredValue::new_local(None);

    provide_context(wb_state.clone());
    provide_context(model);
    provide_context(clipboard);

    // 🚀 **Enhanced Auto-save with Debouncing**
    // Debounced save: waits 2 seconds after the last change before saving
    // Uses enhanced storage with quota checking and better error handling
    let debounced_save = {
        let wb_state = wb_state.clone();
        use_debounce_fn(
            move || {
                let Some(uuid) = wb_state.get_current_uuid_untracked() else {
                    return;
                };
                model.with_value(|m| {
                    // 🚀 Use enhanced storage with better error handling
                    storage_enhanced::save_compatible(&uuid, m);

                    // Optional: Log storage statistics periodically
                    // Use version counter for performance monitoring (legacy compatibility)
                    if wb_state.subscribe_to_any_change()() % 50 == 0 {
                        // Every ~50 changes
                        let analysis = storage_enhanced::analyze_storage();
                        web_sys::console::debug_1(&analysis.into());
                    }
                });
            },
            2000.0, // Save 2 seconds after last change (was 1 second interval)
        )
    };

    // Change detection interval (more frequent checks, less frequent saves)
    // Check every 500ms for changes, but only save via debounced function
    use_interval_fn(
        move || {
            let Some(_uuid) = wb_state.get_current_uuid_untracked() else {
                return;
            };
            let mut has_changes = false;
            model.update_value(|m| {
                has_changes = !m.flush_send_queue().is_empty();
            });
            if has_changes {
                // Trigger debounced save instead of immediate save
                debounced_save();
            }
        },
        500, // Check for changes every 500ms (more responsive)
    );

    // Row layout: collapsible drawer on the left, workbook editor fills the rest.

    view! {
        <div id="app">
            <Workbook />
        </div>
    }
}
