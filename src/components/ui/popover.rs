//! Anchored floating panel — used by domain components that need a panel
//! positioned at absolute viewport coordinates with click-outside dismiss.

use leptos::prelude::*;
use leptos_use::on_click_outside;

/// Floating panel positioned at absolute viewport coordinates, dismissed
/// on click outside the panel.
///
/// Caller owns `open`/`set_open` and `pos`. The panel's CSS class is
/// caller-supplied via `class` so each consumer styles its own surface.
///
/// `above_anchor`: when `true`, renders with `bottom: calc(100vh - y + 4px)`
/// instead of `top: y` - use for menus anchored to a bottom bar.
///
/// # Trigger buttons
/// The button that opens this popover must stop `pointerdown` propagation so
/// `on_click_outside` does not immediately re-close on the same event:
/// `on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()`.
///
/// # Mount strategy
/// Uses `display:none` on a wrapper rather than `<Show when=>` because
/// `children: Children` is `FnOnce` (called once at mount) and `<Show>`
/// requires its children closure to be `Fn`.
#[component]
pub fn Popover(
    open: ReadSignal<bool>,
    set_open: WriteSignal<bool>,
    pos: ReadSignal<(i32, i32)>,
    #[prop(default = false)] above_anchor: bool,
    #[prop(default = "")] class: &'static str,
    children: Children,
) -> impl IntoView {
    let panel_ref = NodeRef::<leptos::html::Div>::new();

    let _ = on_click_outside(panel_ref, move |_| {
        if open.get_untracked() {
            set_open.set(false);
        }
    });

    view! {
        <div style=move || if open.get() { "" } else { "display:none;" }>
            <div
                node_ref=panel_ref
                class=class
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
