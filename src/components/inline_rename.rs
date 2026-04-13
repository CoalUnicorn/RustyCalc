//! Generic inline rename input.
//!
//! A text input that auto-focuses, selects all text, and dispatches
//! commit/cancel via callbacks. Domain-free — callers wire meaning
//! through `on_commit` / `on_cancel` props.
//!
//! ## Usage
//! ```rust
//! let on_commit = Callback::new(move |new_name: String| {
//!     // persist the rename, emit events, etc.
//! });
//! let on_cancel = Callback::new(move |()| {
//!     // revert UI state
//! });
//!
//! <InlineRenameInput
//!     value="Sheet1".to_string()
//!     on_commit=on_commit
//!     on_cancel=on_cancel
//!     class="tab-rename-input"
//! />
//! ```

use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// Inline text input that auto-focuses, selects all, and dispatches via callbacks.
///
/// Caller owns all domain state. The component owns only DOM mechanics:
/// focus/select on mount, Enter/Escape/blur dispatch, and a double-commit
/// guard so `on_commit` never fires twice for the same edit.
///
/// `on_cancel`: called on Escape. When omitted, Escape calls `on_commit`
/// with the original value (effectively a no-op rename).
#[component]
pub fn InlineRenameInput(
    /// Initial text shown in the input (pre-selected on mount).
    value: String,
    /// Called with the new text on Enter or blur.
    on_commit: Callback<String>,
    /// Called on Escape. If omitted, Escape calls `on_commit` with the
    /// original value.
    #[prop(optional)] on_cancel: Option<Callback<()>>,
    /// CSS class for the `<input>` element.
    #[prop(default = "rename-input")] class: &'static str,
) -> impl IntoView {
    let input_ref = NodeRef::<leptos::html::Input>::new();
    let committed = RwSignal::new(false);
    let original = value.clone();

    // Auto-focus and select all text once the element is in the DOM.
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let el2 = el.clone();
            wasm_bindgen_futures::spawn_local(async move {
                el2.focus().ok();
                el2.select();
            });
        }
    });

    let original_for_esc = original.clone();

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        ev.stop_propagation();
        match ev.key().as_str() {
            "Enter" => {
                ev.prevent_default();
                if committed.get_untracked() {
                    return;
                }
                committed.set(true);
                let new_value = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                    .map(|i| i.value())
                    .unwrap_or_default();
                on_commit.run(new_value);
            }
            "Escape" => {
                ev.prevent_default();
                if committed.get_untracked() {
                    return;
                }
                committed.set(true);
                if let Some(cancel) = on_cancel {
                    cancel.run(());
                } else {
                    on_commit.run(original_for_esc.clone());
                }
            }
            _ => {}
        }
    };

    let on_blur = move |ev: web_sys::FocusEvent| {
        if committed.get_untracked() {
            return;
        }
        committed.set(true);
        let new_value = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|i| i.value())
            .unwrap_or_default();
        on_commit.run(new_value);
    };

    view! {
        <input
            node_ref=input_ref
            type="text"
            class=class
            prop:value=value
            on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:mousedown=|ev: web_sys::MouseEvent| ev.stop_propagation()
            on:keydown=on_keydown
            on:blur=on_blur
        />
    }
}
