//! Workbook-sharing controls for the toolbar header: the Share button (encodes
//! the workbook into a `#share=` link), its popover + verification-word input +
//! size-error banner, and the Trust badge for workbooks loaded from a link.

use leptos::prelude::*;

use crate::app_state::AppState;
use crate::components::panels::share_popover::SharePopover;
use crate::state::{ModelStore, WorkbookState};
use crate::storage;

use super::icon::{FileIcon, Icon};

#[component]
pub fn ShareControls() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    let model = expect_context::<ModelStore>();

    // Trust badge appears whenever the current entry's `shared_from_link` flag is
    // set; clicking promotes the workbook and bumps the registry so the sidebar
    // 🔗 badge clears too. Re-reads on `registry_version`.
    let is_shared_current = move || {
        let _ = app.registry_version.get();
        let uuid = state.current_uuid.get()?;
        storage::load_registry()
            .get(&uuid)
            .map(|m| m.shared_from_link)
    };
    let on_trust = move |_: web_sys::MouseEvent| {
        let Some(uuid) = state.current_uuid.get_untracked() else {
            return;
        };
        storage::promote_from_shared(&uuid);
        app.bump_registry();
    };

    let (share_open, set_share_open) = signal(false);
    let share_url = RwSignal::new(String::new());
    let share_error = RwSignal::new(String::new());
    let verify_word = RwSignal::new(String::new());

    let on_share = move |_: web_sys::MouseEvent| {
        let word: Option<String> = {
            let w = verify_word.get();
            let trimmed = w.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        let result = model.with_value(|m| storage::encode_for_share_url(m, word.as_deref()));
        match result {
            Ok(encoded) => {
                let loc = window().location();
                let origin = loc.origin().unwrap_or_default();
                // Include pathname so share links work on sub-path deploys; for
                // opaque origins fall back to the full href minus any hash.
                let base = if origin.is_empty() {
                    loc.href()
                        .unwrap_or_default()
                        .split('#')
                        .next()
                        .unwrap_or("/")
                        .to_string()
                } else {
                    let pathname = loc.pathname().unwrap_or_else(|_| "/".into());
                    format!("{origin}{pathname}")
                };
                share_url.set(format!("{base}#share={encoded}"));
                share_error.set(String::new());
                set_share_open.set(true);
            }
            Err(storage::ShareError::TooLarge { size_kb }) => {
                share_error.set(format!(
                    "This workbook is too large to share via link ({size_kb} KB). \
                     Use File → Download .xlsx instead."
                ));
            }
        }
    };

    view! {
        <Show when=move || is_shared_current().unwrap_or(false)>
            <button
                class="tb-trust"
                title="This workbook was loaded from a shared link. Click to trust it and clear the badge."
                on:click=on_trust
            >
                "🛡 Trust"
            </button>
        </Show>

        <button class="tb-btn tb-share" title="Share via link" on:click=on_share>
            <Icon icon=FileIcon::Share />
        </button>

        <Show when=move || share_open.get()>
            <SharePopover
                share_url=share_url.get()
                verify_word=verify_word.get()
                on_close=Callback::new(move |_| set_share_open.set(false))
            />
        </Show>

        <Show when=move || !share_error.get().is_empty()>
            <div class="sp-error-banner">{move || share_error.get()}</div>
        </Show>

        <Show when=move || share_open.get()>
            <div class="sp-word-row">
                <input
                    type="text"
                    class="sp-word-input"
                    placeholder="Verification word (optional, 3+ letters)"
                    prop:value=verify_word
                    on:input=move |ev| verify_word.set(event_target_value(&ev))
                />
            </div>
        </Show>
    }
}
