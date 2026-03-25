use crate::components::workbook::Workbook;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

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

    provide_context(wb_state.clone());
    provide_context(model);
    provide_context(clipboard);

    // ── Auto-save interval ────────────────────────────────────────────────────
    // Every second, flush the model's pending diff queue. If there are unsaved
    // mutations, persist to localStorage. `cb.forget()` is intentional — the
    // closure lives for the duration of the app (no way to cancel it anyway).
    {
        let save_state = wb_state;
        let save_model = model;
        let cb = Closure::wrap(Box::new(move || {
            let Some(uuid) = save_state.current_uuid.get_untracked() else {
                return;
            };
            let mut has_changes = false;
            save_model.update_value(|m| {
                has_changes = !m.flush_send_queue().is_empty();
            });
            if has_changes {
                save_model.with_value(|m| storage::save(&uuid, m));
            }
        }) as Box<dyn Fn()>);
        #[allow(clippy::expect_used)]
        let win = web_sys::window().expect("window must exist in WASM context");
        #[allow(clippy::expect_used)]
        win.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            1000,
        )
        .expect("set_interval must not fail");
        cb.forget();
    }

    // Row layout: collapsible drawer on the left, workbook editor fills the rest.

    view! {
        <div
            id="app"
            style="width:100vw;height:100vh;display:flex;flex-direction:row;overflow:hidden;\
                font-family:Inter,Arial,sans-serif;"
        >
            <Workbook />
        </div>
    }
}
