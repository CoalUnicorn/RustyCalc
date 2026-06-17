use leptos::prelude::*;

use crate::app_state::AppState;
use crate::components::ui::modal::Modal;
use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::state::{ModelStore, WorkbookState};
use crate::storage::{self, SharedLoad};

/// Maximum verification attempts before locking out the V1 word entry.
const MAX_ATTEMPTS: u32 = 3;

/// Consent modal shown when a `#share=…` URL is detected on startup.
///
/// Bitcode parsing is deferred until the user accepts, so a crafted payload
/// can't trigger expensive deserialization on first paint. V0 (no word) shows
/// a simple accept/reject with the payload size; V1 (word-verified) requires
/// the recipient to type the sender's word before parse + persist run.
#[component]
pub fn ShareVerify() -> impl IntoView {
    let pending = expect_context::<ReadSignal<Option<SharedLoad>>>();
    let set_pending = expect_context::<WriteSignal<Option<SharedLoad>>>();
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();
    let app = expect_context::<AppState>();

    let word = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let attempts = RwSignal::new(0u32);
    let loading = RwSignal::new(false);

    let show = move || pending.get().is_some();

    let reset_modal = move || {
        word.set(String::new());
        error.set(String::new());
        attempts.set(0);
        loading.set(false);
    };

    // After a successful accept (V0 or V1), swap the active workbook in
    // place: persist the staged model under a fresh UUID, mark it as
    // current, clear the URL hash, and emit a content event so the canvas
    // repaints.
    let activate_shared = move |loaded: ironcalc_base::UserModel<'static>| {
        if let Some(prev) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&prev, m));
        }
        let (new_uuid, new_model) =
            storage::create_new_from(loaded, storage::WorkbookOrigin::ShareLink);
        model.update_value(|m| *m = new_model);
        state.current_uuid.set(Some(new_uuid));
        state.reset_view_state();
        state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));
        app.bump_registry();
        let _ = window().location().set_hash("");
        set_pending.set(None);
        reset_modal();
    };

    let close_modal = move || {
        let _ = window().location().set_hash("");
        set_pending.set(None);
        reset_modal();
    };

    let on_dismiss = move |_: leptos::ev::MouseEvent| {
        close_modal();
    };

    let on_accept_v0 = move |_: leptos::ev::MouseEvent| {
        let Some(SharedLoad::PendingV0 { bytes }) = pending.get() else {
            return;
        };
        loading.set(true);
        match storage::accept_shared_v0(&bytes) {
            Ok(m) => activate_shared(m),
            Err(e) => {
                error.set(e);
                loading.set(false);
            }
        }
    };

    let on_submit_v1 = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some(SharedLoad::PendingV1 { hash, bytes }) = pending.get() else {
            return;
        };
        loading.set(true);
        let w = word.get();
        match storage::verify_and_load_shared(&hash, &w, &bytes) {
            Ok(m) => activate_shared(m),
            Err(e) => {
                let n = attempts.get() + 1;
                attempts.set(n);
                if n >= MAX_ATTEMPTS {
                    error.set("Too many attempts. The workbook was not loaded.".into());
                } else {
                    error.set(format!("{e} ({n}/{MAX_ATTEMPTS} attempts)"));
                }
                loading.set(false);
            }
        }
    };

    let size_kb = move || {
        pending
            .get()
            .map(|s| (s.size_bytes() as f64 / 1024.0).max(0.1))
            .unwrap_or(0.0)
    };

    let is_v1 = move || matches!(pending.get(), Some(SharedLoad::PendingV1 { .. }));

    let can_submit_v1 = move || {
        !loading.get() && word.get().trim().chars().count() >= 3 && attempts.get() < MAX_ATTEMPTS
    };
    let locked = move || attempts.get() >= MAX_ATTEMPTS;

    view! {
        <Show when=show>
            <Modal title="Open shared workbook?" on_close=Callback::new(move |_| {
                close_modal();
            })>
                <div class="sv-container">
                    <p class="sv-description">
                        "Source: Shared from URL · "
                        {move || format!("{:.1} KB", size_kb())}
                    </p>
                    <Show when=is_v1
                        fallback=move || view! {
                            <p class="sv-description">
                                "This workbook was sent to you via a link. Open it now? \
                                 The content is not parsed until you accept."
                            </p>
                            <div class="sv-input-row">
                                <button
                                    class="sv-submit-btn"
                                    disabled=move || loading.get()
                                    on:click=on_accept_v0
                                >
                                    {move || if loading.get() { "Opening..." } else { "Open Workbook" }}
                                </button>
                                <button class="sv-dismiss-btn" on:click=on_dismiss>
                                    "Reject"
                                </button>
                            </div>
                        }
                    >
                        <p class="sv-description">
                            "This workbook is shared with verification. \
                             Enter the word the sender gave you:"
                        </p>
                        <form on:submit=on_submit_v1>
                            <div class="sv-input-row">
                                <input
                                    type="text"
                                    class="sv-word-input"
                                    placeholder="Type verification word..."
                                    autofocus
                                    disabled=locked
                                    prop:value=word
                                    on:input=move |ev| {
                                        word.set(event_target_value(&ev));
                                        if !error.get().is_empty() {
                                            error.set(String::new());
                                        }
                                    }
                                />
                                <button
                                    type="submit"
                                    class="sv-submit-btn"
                                    disabled=move || !can_submit_v1()
                                >
                                    {move || if loading.get() { "Loading..." } else { "Open Workbook" }}
                                </button>
                            </div>
                        </form>
                    </Show>
                    <Show when=move || !error.get().is_empty()>
                        <div class="sv-error">{move || error.get()}</div>
                    </Show>
                    <button
                        class="sv-dismiss-btn"
                        style:display=move || if locked() { "block" } else { "none" }
                        on:click=on_dismiss
                    >
                        "Dismiss"
                    </button>
                </div>
            </Modal>
        </Show>
    }
}
