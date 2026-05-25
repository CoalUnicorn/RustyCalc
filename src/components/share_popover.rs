//! Share popover — shows the URL for the current workbook and a Copy button.
//!
//! Hosted by [`crate::components::file_bar::FileBar`] inside a `<Show when=open>`
//! gate. Re-mounts each time it's opened, so `share_url` is captured fresh
//! from the host on every open and we don't need a reactive prop.

use std::time::Duration;

use leptos::prelude::*;

use crate::components::modal::Modal;

/// Modal popover that exposes a copyable share URL.
///
/// `share_url` is taken by value — the host (FileBar) rebuilds it on demand
/// when the user opens the popover, then unmounts the component on close,
/// so a static String is the simplest contract.
#[component]
pub fn SharePopover(share_url: String, on_close: Callback<()>) -> impl IntoView {
    // "Copied!" flash. Two-second auto-revert so the user gets feedback
    // without a permanent state change.
    let copied = RwSignal::new(false);

    // Clone once per consumer — closures need owned Strings to stay 'static,
    // and `prop:value` needs its own copy that outlives the click handler.
    let url_for_input = share_url.clone();
    let url_for_copy = share_url;

    let handle_copy = move |_| {
        // Best-effort clipboard write. The returned Promise is intentionally
        // dropped: failure is silent (sandboxed iframes, denied permissions)
        // and the URL stays visible in the input as a manual fallback.
        let _ = window().navigator().clipboard().write_text(&url_for_copy);
        copied.set(true);
        set_timeout(move || copied.set(false), Duration::from_secs(2));
    };

    let copy_label = move || if copied.get() { "Copied!" } else { "Copy URL" };

    view! {
        <Modal title="Share this workbook" on_close=on_close>
            <div class="sp-popover">
                <div class="sp-url-row">
                    <input
                        type="text"
                        class="sp-url-input"
                        readonly
                        prop:value=url_for_input
                    />
                    <button class="sp-copy-btn" on:click=handle_copy>
                        {copy_label}
                    </button>
                </div>
                <div class="sp-footer">
                    "Anyone with the link can access a copy of this workbook. \
                     The link contains the entire workbook — treat it like the .xlsx file itself."
                </div>
            </div>
        </Modal>
    }
}
