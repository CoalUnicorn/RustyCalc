//! Share popover — shows the URL for the current workbook, an editable
//! verification word, and a Copy button.
//!
//! Hosted by `ShareControls`
//! inside a `<Show when=open>` gate.

use std::time::Duration;

use leptos::prelude::*;

use crate::components::ui::modal::Modal;

/// Modal popover exposing a copyable share URL and an optional verification word.
///
/// `share_url` and `verify_word` are reactive signals owned by the host
/// (ShareControls). They're signals rather than plain Strings because the word
/// is editable here: committing it (blur/Enter) runs `regenerate`, which
/// re-encodes the workbook so `share_url` — and thus the copied link — reflects
/// the word. Per-keystroke `on:input` only updates the cheap word signal; the
/// expensive re-encode is deferred to `on:change`.
#[component]
pub fn SharePopover(
    share_url: RwSignal<String>,
    verify_word: RwSignal<String>,
    regenerate: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    // "Copied!" flash. Two-second auto-revert so the user gets feedback
    // without a permanent state change.
    let copied = RwSignal::new(false);

    let handle_copy = move |_| {
        // Best-effort clipboard write. The returned Promise is intentionally
        // dropped: failure is silent (sandboxed iframes, denied permissions)
        // and the URL stays visible in the input as a manual fallback.
        let _ = window()
            .navigator()
            .clipboard()
            .write_text(&share_url.get_untracked());
        copied.set(true);
        set_timeout(move || copied.set(false), Duration::from_secs(2));
    };

    let copy_label = move || if copied.get() { "Copied!" } else { "Copy URL" };

    view! {
        <Modal title="Share this workbook" on_close=on_close>
            <div class="sp-popover">
                <div class="sp-word-row">
                    <input
                        type="text"
                        class="sp-word-input"
                        placeholder="Verification word (optional, 3+ letters)"
                        prop:value=verify_word
                        on:input=move |ev| verify_word.set(event_target_value(&ev))
                        on:change=move |_| { regenerate.run(()); }
                    />
                </div>
                <p class="sp-verify-hint">
                    "Set a word and share it with the receiver out-of-band — \
                     they'll need to type it to open the workbook. The link below \
                     updates when you commit the word."
                </p>
                <div class="sp-url-row">
                    <input type="text" class="sp-url-input" readonly prop:value=share_url />
                    <button class="sp-copy-btn" on:click=handle_copy>
                        {copy_label}
                    </button>
                </div>
                <div class="sp-footer">
                    "The link contains the entire workbook — anyone with it can \
                     access a copy. The link may be saved in your browser history \
                     and synced across devices."
                </div>
            </div>
        </Modal>
    }
}
