//! Generic modal dialog primitive.
//!
//! Owns the structural pieces every modal needs and nothing else:
//! - a full-viewport backdrop that swallows pointer events,
//! - Esc-to-close via a document-level `keydown` listener (so a focused
//!   `<select>` popup or any other element that steals focus can't eat
//!   the key),
//! - click-outside-to-close via `leptos_use::on_click_outside` on the
//!   inner box (so a click on the backdrop closes; clicks inside the box
//!   bubble normally),
//! - an `on_close` callback the host invokes from its own buttons (Cancel,
//!   the X icon, post-save flow, etc.).
//!
//! The host is responsible for *mounting* the modal conditionally
//! (`<Show when=is_open>{ <Modal …/> }</Show>`) — this component does not
//! own the open/closed signal. The document-level listener registers only
//! while the component is mounted (leptos_use unbinds it on owner drop).

use leptos::ev::keydown;
use leptos::prelude::*;
use leptos_use::{on_click_outside, use_document, use_event_listener};

/// Sizing modifier applied to the inner `.modal-box`. Maps to a CSS class so
/// width/height tuning lives in stylesheets, not Rust string formatting.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ModalSize {
    Small,
    Medium,
    Large,
}

impl ModalSize {
    fn css_class(self) -> &'static str {
        match self {
            ModalSize::Small => "md-sm",
            ModalSize::Medium => "md-md",
            ModalSize::Large => "md-lg",
        }
    }
}

/// Generic modal scaffold. Renders a backdrop + a sized inner box containing
/// `children()`. Keeps no internal state.
///
/// `title` is rendered into the dialog header next to the close button.
/// `on_close` fires on backdrop click, Esc keydown, or close-button click.
#[component]
pub fn Modal(
    #[prop(into)] title: String,
    on_close: Callback<()>,
    #[prop(default = ModalSize::Medium)] size: ModalSize,
    children: Children,
) -> impl IntoView {
    // The box is the click-outside target, not the backdrop, so a click on
    // any blank backdrop region closes the modal — matches the user's
    // expectation that the dim area is not interactive.
    let box_ref = NodeRef::<leptos::html::Div>::new();
    let close = move || on_close.run(());
    let _ = on_click_outside(box_ref, move |_| close());

    // Esc closes from anywhere in the document — robust against a child
    // component (a `<select>` popup, a context menu) holding focus when
    // the user hits Esc.
    let _ = use_event_listener(
        use_document(),
        keydown,
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Escape" {
                ev.prevent_default();
                close();
            }
        },
    );

    // Move focus into the dialog on mount so keyboard and screen-reader
    // users land inside the modal, not on the page beneath.
    Effect::new(move |_| {
        if let Some(el) = box_ref.get() {
            el.focus().ok();
        }
    });

    let on_close_btn = move |_: web_sys::MouseEvent| close();

    let box_class = format!("md-box {}", size.css_class());

    view! {
        <div class="md-backdrop">
            <div class=box_class node_ref=box_ref tabindex="-1">
                <div class="md-header">
                    <span class="md-title">{title}</span>
                    <button class="md-close" on:click=on_close_btn title="Close (Esc)">"✕"</button>
                </div>
                <div class="md-body">
                    {children()}
                </div>
            </div>
        </div>
    }
}
