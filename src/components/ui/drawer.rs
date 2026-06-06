//! Generic right-side drawer primitive.
//!
//! A non-modal sibling to [`super::modal::Modal`]. Where `Modal` dims and
//! blocks the page behind a pointer-swallowing backdrop, the drawer is a
//! panel pinned to the right edge that leaves the grid fully live — the user
//! can keep clicking cells while it is open (that is the whole point: range
//! fields inside the drawer capture grid selections).
//!
//! Consequently it deliberately omits two of Modal's behaviours:
//! - **No backdrop** — nothing covers the grid, so grid clicks reach the canvas.
//! - **No click-outside-to-close** — a click on the grid is a range pick, not a
//!   dismissal; closing is only ever explicit (X button, Esc, host buttons).
//!
//! It keeps Modal's **document-level Esc listener**: the workbook keydown
//! router early-returns when an `input`/`textarea`/`select` is focused
//! (`workbook/mod.rs`), so a focused drawer field would otherwise swallow Esc.
//! Registering on the document sidesteps that. The host decides what Esc means
//! via `on_close` (e.g. disarm a range pick first, then close).
//!
//! Like `Modal`, the host mounts it conditionally
//! (`<Show when=is_open>{ <Drawer …/> }</Show>`) and owns the open signal.

use leptos::ev::keydown;
use leptos::prelude::*;
use leptos_use::{use_document, use_event_listener};

/// Width modifier applied to the `.drawer` panel. Maps to a CSS class so the
/// actual sizing lives in the stylesheet, mirroring [`super::modal::ModalSize`].
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerWidth {
    Small,
    Medium,
    Large,
}

impl DrawerWidth {
    fn css_class(self) -> &'static str {
        match self {
            DrawerWidth::Small => "drawer-sm",
            DrawerWidth::Medium => "drawer-md",
            DrawerWidth::Large => "drawer-lg",
        }
    }
}

/// Right-pinned, non-modal drawer scaffold. Renders a header (title + close)
/// over a scrollable body containing `children()`. Keeps no internal state.
///
/// `on_close` fires on the close-button click or an Escape keydown anywhere in
/// the document. It does **not** fire on grid/outside clicks (by design).
#[component]
pub fn Drawer(
    #[prop(into)] title: String,
    on_close: Callback<()>,
    #[prop(default = DrawerWidth::Medium)] width: DrawerWidth,
    children: Children,
) -> impl IntoView {
    let close = move || on_close.run(());

    // Esc closes from anywhere in the document — robust against a focused
    // drawer field (whose keydown the workbook router would otherwise ignore).
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

    let on_close_btn = move |_: web_sys::MouseEvent| close();

    let panel_class = format!("drawer {}", width.css_class());

    view! {
        <div class=panel_class>
            <div class="drawer-header">
                <span class="drawer-title">{title}</span>
                <button class="drawer-close" on:click=on_close_btn title="Close (Esc)">"✕"</button>
            </div>
            <div class="drawer-body">
                {children()}
            </div>
        </div>
    }
}
