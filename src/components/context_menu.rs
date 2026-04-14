//! Generic context menu components.
//!
//! ## Usage
//! ```rust
//! // Caller owns signals:
//! let (open, set_open) = signal(false);
//! let (pos,  set_pos)  = signal((0i32, 0i32));
//!
//! // Button-triggered:
//! <ContextMenuButton {set_open} {set_pos}>"Open menu"</ContextMenuButton>
//! <ContextMenu {open} {set_open} {pos}>
//!     <ContextMenuItem on_click=|| log("action")>"Do thing"</ContextMenuItem>
//!     <ContextMenuSeparator />
//!     <ContextMenuItem on_click=|| {} destructive=true>"Delete"</ContextMenuItem>
//! </ContextMenu>
//!
//! // Right-click (no button):
//! // on contextmenu event: set_pos((ev.client_x(), ev.client_y())); set_open(true); ev.prevent_default();
//! ```

use leptos::prelude::*;
use leptos_use::on_click_outside;

/// Dropdown container.  Caller owns `open`/`set_open` and `pos`/`set_pos`.
///
/// Provides `set_open: WriteSignal<bool>` via context so [`ContextMenuItem`]
/// children can close the menu automatically.
///
/// `above_anchor`: when `true`, renders with `bottom: calc(100vh - y + 4px)`
/// instead of `top: y` - use for menus anchored to a bottom bar.
///
/// # Trigger buttons
/// The button that opens this menu must stop `pointerdown` propagation so
/// `on_click_outside` does not immediately re-close the menu on the same
/// event: `on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()`.
#[component]
pub fn ContextMenu(
    open: ReadSignal<bool>,
    set_open: WriteSignal<bool>,
    pos: ReadSignal<(i32, i32)>,
    #[prop(default = false)] above_anchor: bool,
    children: Children,
) -> impl IntoView {
    provide_context(set_open);

    let menu_ref = NodeRef::<leptos::html::Div>::new();

    // Close when the user clicks/taps anywhere outside the menu panel.
    // Guard against spurious fires when the menu is already closed.
    let _ = on_click_outside(menu_ref, move |_| {
        if open.get_untracked() {
            set_open.set(false);
        }
    });

    // `children()` is FnOnce - must be called exactly once at mount time.
    // We use `display:none` on a wrapper div rather than `<Show>` to avoid
    // the Leptos 0.7 constraint that Show's children closure must be `Fn`.
    view! {
        <div style=move || if open.get() { "" } else { "display:none;" }>
            <div
                node_ref=menu_ref
                class="ctx"
                style=move || {
                    let (x, y) = pos.get();
                    if above_anchor {
                        format!("left:{x}px;bottom:calc(100vh - {y}px + 4px);")
                    } else {
                        format!("left:{x}px;top:{y}px;")
                    }
                }
            >
                {children()}
            </div>
        </div>
    }
}

/// Convenience trigger button that records click coords and toggles `open`.
///
/// For right-click or custom triggers, wire `set_open` / `set_pos` directly.
/// Unused until the file bar and cell right-click context menus are wired up.
#[allow(dead_code)]
#[component]
pub fn ContextMenuButton(
    set_open: WriteSignal<bool>,
    set_pos: WriteSignal<(i32, i32)>,
    #[prop(default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    let on_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        set_pos.set((ev.client_x(), ev.client_y()));
        set_open.update(|v| *v = !*v);
    };

    view! {
        <span class=class on:click=on_click>
            {children()}
        </span>
    }
}

/// A single menu action row.
///
/// Reads `WriteSignal<bool>` from context (provided by [`ContextMenu`]) and
/// calls `set_open(false)` after `on_click` - so items always close the menu.
///
/// `icon`: optional emoji / text shown in a fixed-width span.
/// `destructive`: adds `.destructive` CSS modifier (red text).
#[component]
pub fn ContextMenuItem(
    on_click: impl Fn() + Send + Sync + 'static,
    #[prop(optional)] icon: Option<&'static str>,
    #[prop(default = false)] destructive: bool,
    children: Children,
) -> impl IntoView {
    let set_open = use_context::<WriteSignal<bool>>();
    #[cfg(debug_assertions)]
    if set_open.is_none() {
        web_sys::console::warn_1(&"ContextMenuItem rendered without a ContextMenu ancestor".into());
    }

    // Arc so the closure can be called via Fn (not FnOnce) inside the event handler.
    let on_click = std::sync::Arc::new(on_click);

    let handle_click = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        on_click();
        if let Some(close) = set_open {
            close.set(false);
        }
    };

    // `destructive` is a static bool prop - use a plain format string, not a
    // reactive closure, to avoid registering a spurious reactive dependency.
    let class = format!("ctx-item{}", if destructive { " destructive" } else { "" });

    view! {
        <div
            class=class
            on:click=handle_click
        >
            {icon.map(|i| view! { <span class="ctx-icon">{i}</span> })}
            {children()}
        </div>
    }
}

/// Horizontal divider between menu sections.
#[component]
pub fn ContextMenuSeparator() -> impl IntoView {
    view! { <div class="ctx-div" /> }
}
