use crate::components::left_drawer::LeftDrawer;
use crate::components::workbook::Workbook;

use leptos::prelude::*;
use leptos_use::{use_debounce_fn_with_options, DebounceOptions};

use crate::app_state::AppState;
use crate::events::EventBus;
use crate::state::WorkbookState;
use crate::storage;

#[component]
pub fn App() -> impl IntoView {
    // Load the previously selected workbook from localStorage, or create a
    // fresh blank one if localStorage is empty (first launch).
    let (uuid, model) = storage::load_selected().unwrap_or_else(storage::create_new);

    let events = EventBus::new();
    // AppState owns the leptos-use color-mode handles internally; theme
    // changes flow `app.set_theme -> set_mode -> <html data-theme>` with
    // no app-level bridging Effect required.
    let app_state = AppState::new(events);
    let wb_state = WorkbookState::new(events);
    wb_state.current_uuid.set(Some(uuid));

    let model = StoredValue::new_local(model);

    // Internal clipboard - mirrors what was last copied/cut, so Ctrl+V can
    // paste even if the OS clipboard is unavailable (sandboxed iframe, etc.).
    let clipboard: StoredValue<Option<crate::model::AppClipboard>, LocalStorage> =
        StoredValue::new_local(None);

    provide_context(app_state);
    // Provide PerfTimings independently so `try_mutate` can write phase
    // timestamps without coupling the model layer to AppState.
    provide_context(app_state.perf);
    provide_context(wb_state);
    provide_context(model);
    provide_context(clipboard);

    // Centralized auto-save via EventBus subscription.
    //
    // Every model mutation emits events for UI updates. This Effect subscribes
    // to the three mutation categories (content, format, structure) and triggers
    // a debounced save. Navigation and theme events are ephemeral — no persistence.
    //
    // Timing: 1s after last change, max 5s during continuous edits.
    // Safety net: beforeunload saves unconditionally on tab close.
    // Lifecycle: workbook switch saves the outgoing model synchronously
    //            in input/workbook.rs before model replacement.
    let debounced_save = use_debounce_fn_with_options(
        move || {
            if let Some(uuid) = wb_state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
        },
        1000.0,
        DebounceOptions::default().max_wait(Some(5000.0)),
    );

    Effect::new(move |_| {
        let has_content = !wb_state.events.content.get().is_empty();
        let has_format = !wb_state.events.format.get().is_empty();
        let has_structure = !wb_state.events.structure.get().is_empty();
        if has_content || has_format || has_structure {
            debounced_save();
        }
    });

    // Emergency save on tab close — unconditional, cheap, runs rarely.
    {
        use wasm_bindgen::prelude::*;
        let cb = Closure::<dyn Fn(web_sys::Event)>::new(move |_: web_sys::Event| {
            if let Some(uuid) = wb_state.current_uuid.get_untracked() {
                model.with_value(|m| storage::save(&uuid, m));
            }
        });
        if let Ok(win) =
            window().add_event_listener_with_callback("beforeunload", cb.as_ref().unchecked_ref())
        {
            // Listener registered; intentionally leak the closure so it
            // lives until page unload.
            win
        }
        cb.forget();
    }

    // Row layout: collapsible drawer on the left, workbook editor fills the rest.

    view! {
        <div id="app">
            <LeftDrawer />
            <Workbook />
        </div>
    }
}
