use leptos::prelude::*;

use crate::components::modal::Modal;
use crate::storage;

/// Maximum verification attempts before locking out.
const MAX_ATTEMPTS: u32 = 3;

#[component]
pub fn ShareVerify() -> impl IntoView {
    let pending_share =
        expect_context::<ReadSignal<Option<(String, Vec<u8>, [u8; 32])>>>();
    let set_pending_share =
        expect_context::<WriteSignal<Option<(String, Vec<u8>, [u8; 32])>>>();

    let word = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let attempts = RwSignal::new(0u32);
    let loading = RwSignal::new(false);

    let show = move || pending_share.get().is_some();

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Some((_hash_str, bytes, hash)) = pending_share.get() else {
            return;
        };
        loading.set(true);
        let w = word.get();
        match storage::verify_and_load_shared(&hash, &w, &bytes) {
            Ok(model) => {
                storage::create_new_from(model);
                let _ = window().location().set_hash("");
                set_pending_share.set(None);
                word.set(String::new());
                error.set(String::new());
                attempts.set(0);
            }
            Err(e) => {
                let n = attempts.get() + 1;
                attempts.set(n);
                if n >= MAX_ATTEMPTS {
                    error.set(
                        "Too many attempts. The workbook was not loaded."
                            .into(),
                    );
                } else {
                    error.set(format!("{e} ({n}/{MAX_ATTEMPTS} attempts)"));
                }
            }
        }
        loading.set(false);
    };

    let on_dismiss = move |_: leptos::ev::MouseEvent| {
        let _ = window().location().set_hash("");
        set_pending_share.set(None);
        word.set(String::new());
        error.set(String::new());
        attempts.set(0);
    };

    let can_submit = move || {
        !loading.get() && word.get().trim().len() >= 3 && attempts.get() < MAX_ATTEMPTS
    };

    let locked = move || attempts.get() >= MAX_ATTEMPTS;

    view! {
        <Show when=show>
            <Modal title="Verify shared workbook" on_close=Callback::new(move |_| {
                let _ = window().location().set_hash("");
                set_pending_share.set(None);
            })>
                <div class="sv-container">
                    <p class="sv-description">
                        "This workbook is shared with verification. \
                         Enter the word the sender gave you:"
                    </p>
                    <form on:submit=on_submit>
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
                                disabled=move || !can_submit()
                            >
                                {move || if loading.get() { "Loading..." } else { "Open Workbook" }}
                            </button>
                        </div>
                    </form>
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
