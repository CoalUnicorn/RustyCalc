use crate::components::chrome::left_drawer::LeftDrawer;
use crate::components::panels::share_verify::ShareVerify;
use crate::components::workbook::Workbook;

use base64::Engine;
use gloo_storage::Storage;
use leptos::prelude::*;
use leptos_use::{DebounceOptions, use_debounce_fn_with_options};

use crate::app_state::AppState;
use crate::events::EventBus;
use crate::state::WorkbookState;
use crate::storage;

#[component]
pub fn App() -> impl IntoView {
    // `#share=...` URLs are always staged in-memory pending user consent —
    // never auto-persisted on first paint. Both V0 (no word) and V1 (word
    // verification) defer bitcode deserialization until after the recipient
    // clicks Accept in the ShareVerify modal, so a crafted payload can't
    // run the parser on page load. The URL hash stays intact while the
    // modal is open so a refresh re-enters this same branch; it's cleared
    // on accept or dismiss.
    let (pending_share, set_pending_share) = signal(None::<storage::SharedLoad>);
    let share_load = storage::load_shared_from_url();
    if let Some(load) = share_load {
        set_pending_share.set(Some(load));
    }
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

    // Pre-serialized model bytes refreshed by the debounced save. The
    // beforeunload handler reads these directly, sidestepping a 10-50ms
    // bitcode pass inside the browser's ~200ms unload budget.
    let pre_serialized: StoredValue<Option<Vec<u8>>, LocalStorage> = StoredValue::new_local(None);

    provide_context(app_state);
    // Provide PerfTimings independently so `try_mutate` can write phase
    // timestamps without coupling the model layer to AppState.
    provide_context(app_state.perf);
    provide_context(wb_state);
    provide_context(model);
    provide_context(clipboard);
    // Pending v1 share verification — consumed by ShareVerify modal.
    provide_context(pending_share);
    provide_context(set_pending_share);

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
            let Some(uuid) = wb_state.current_uuid.get_untracked() else {
                return;
            };
            model.with_value(|m| {
                storage::save(&uuid, m);
                web_sys::console::time_with_label("bitcode::to_bytes");
                let bytes = m.to_bytes();
                web_sys::console::time_end_with_label("bitcode::to_bytes");
                pre_serialized.set_value(Some(bytes));
            });
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
            let Some(uuid) = wb_state.current_uuid.get_untracked() else {
                return;
            };
            // Fast path: write the bytes the idle save already produced,
            // bypassing a fresh bitcode pass. Registry was updated then too,
            // so we only need to flush the model payload here.
            if let Some(bytes) = pre_serialized.get_value() {
                let mut full = Vec::with_capacity(5 + bytes.len());
                full.extend_from_slice(b"RCAL");
                full.push(1u8);
                full.extend_from_slice(&bytes);
                let encoded = base64::engine::general_purpose::STANDARD.encode(&full);
                storage::log_err(
                    gloo_storage::LocalStorage::set(uuid.to_string(), &encoded),
                    "beforeunload save",
                );
            } else {
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
            <ShareVerify />
        </div>
    }
}
