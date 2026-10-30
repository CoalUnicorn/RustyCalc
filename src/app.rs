use crate::components::chrome::left_drawer::LeftDrawer;
use crate::components::panels::share_verify::ShareVerify;
use crate::components::workbook::Workbook;

use leptos::prelude::*;
use leptos_use::{DebounceOptions, use_debounce_fn_with_options};

use crate::app_state::AppState;
use crate::events::EventBus;
use crate::input::mouse::CanvasHandle;
use crate::state::WorkbookState;
use crate::storage;

#[component]
pub fn App(initial: (storage::WorkbookId, ironcalc_base::UserModel<'static>)) -> impl IntoView {
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
    let (uuid, model) = initial;

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

    // Scene render handle. App-level (like `clipboard`) because the toolbar
    // and the hover tooltip read committed canvas state (links, cell rects)
    // while the grid canvas is mounted by `Worksheet`, a sibling. `Worksheet`
    // fills the slot on mount and empties it on cleanup.
    let canvas_handle: CanvasHandle = StoredValue::new_local(None::<crate::scene::SceneHandle>);

    provide_context(app_state);
    provide_context(wb_state);
    provide_context(model);
    provide_context(clipboard);
    provide_context(canvas_handle);
    // Pending v1 share verification — consumed by ShareVerify modal.
    provide_context(pending_share);
    provide_context(set_pending_share);

    let generation = RwSignal::new(0u64);
    let saved_generation = RwSignal::new(0u64);
    let pending_writes = RwSignal::new(0u32);
    let save_current = move || {
        if app_state.storage_busy.get_untracked() {
            return;
        }
        let Some(uuid) = wb_state.current_uuid.get_untracked() else {
            return;
        };
        let version = generation.get_untracked();
        let snapshot = model.with_value(|m| storage::snapshot(uuid, m));
        pending_writes.update(|count| *count += 1);
        wasm_bindgen_futures::spawn_local(async move {
            let result = match snapshot {
                Ok(snapshot) => snapshot.save().await,
                Err(error) => Err(error),
            };
            pending_writes.update(|count| *count -= 1);
            match result {
                Ok(()) => {
                    if wb_state.current_uuid.get_untracked() == Some(uuid) {
                        saved_generation.update(|saved| *saved = (*saved).max(version));
                    }
                }
                Err(error) => wb_state
                    .status
                    .set(Some(crate::state::StatusMessage::Error(error))),
            }
        });
    };
    let debounced_save = use_debounce_fn_with_options(
        save_current,
        1000.0,
        DebounceOptions::default().max_wait(Some(5000.0)),
    );
    let save_after_operation = debounced_save.clone();
    Effect::new(move |_| {
        let has_content = !wb_state.events.content.get().is_empty();
        let has_format = !wb_state.events.format.get().is_empty();
        let has_structure = !wb_state.events.structure.get().is_empty();
        if has_content || has_format || has_structure {
            generation.update(|value| *value += 1);
            debounced_save();
        }
    });
    Effect::new(move |_| {
        if !app_state.storage_busy.get()
            && generation.get_untracked() != saved_generation.get_untracked()
        {
            save_after_operation();
        }
    });

    // Start a current snapshot while the page is still alive. Unload cannot await IndexedDB.
    {
        use wasm_bindgen::prelude::*;
        let hidden = Closure::<dyn Fn(web_sys::Event)>::new(move |_| {
            if document().hidden() && generation.get_untracked() != saved_generation.get_untracked()
            {
                save_current();
            }
        });
        let unload = Closure::<dyn Fn(web_sys::Event)>::new(move |event: web_sys::Event| {
            if generation.get_untracked() != saved_generation.get_untracked()
                || pending_writes.get_untracked() > 0
                || app_state.storage_busy.get_untracked()
            {
                event.prevent_default();
                let _ = js_sys::Reflect::set(event.as_ref(), &"returnValue".into(), &"".into());
            }
        });
        let _ = document()
            .add_event_listener_with_callback("visibilitychange", hidden.as_ref().unchecked_ref());
        let _ = window()
            .add_event_listener_with_callback("beforeunload", unload.as_ref().unchecked_ref());
        let listeners = StoredValue::new_local((hidden, unload));
        on_cleanup(move || {
            listeners.with_value(|(hidden, unload)| {
                let _ = document().remove_event_listener_with_callback(
                    "visibilitychange",
                    hidden.as_ref().unchecked_ref(),
                );
                let _ = window().remove_event_listener_with_callback(
                    "beforeunload",
                    unload.as_ref().unchecked_ref(),
                );
            })
        });
    }

    // Row layout: collapsible drawer on the left, workbook editor fills the rest.

    view! {
        <div id="app" inert=move || app_state.storage_busy.get() aria-busy=move || app_state.storage_busy.get().to_string()>
            <LeftDrawer />
            <Workbook />
            <ShareVerify />
        </div>
    }
}
