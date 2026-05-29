//! Anchored floating panel — used by domain components that need a panel
//! positioned at absolute viewport coordinates with click-outside dismiss.

use leptos::prelude::*;
use leptos_use::{
    UseElementSizeReturn, UseWindowSizeReturn, on_click_outside, use_element_size, use_window_size,
};

/// Floating panel positioned at absolute viewport coordinates, dismissed
/// on click outside the panel.
///
/// Caller owns `open`/`set_open` and `pos`. The panel's CSS class is
/// caller-supplied via `class` so each consumer styles its own surface.
///
/// `above_anchor`: when `true`, the panel grows upward from `pos.y` (anchored
/// by its bottom edge) instead of downward — use for menus anchored to a
/// bottom bar.
///
/// # Viewport clamping
/// Positioning is **edge-aware**: the panel measures itself
/// ([`use_element_size`]) and the viewport ([`use_window_size`]) and clamps
/// `left`/`top`/`bottom` so it never overflows the window. Clamping is
/// reactive — it re-runs when the panel's content grows (e.g. an inline color
/// picker expanding inside a context menu) or the window resizes. Because all
/// floating chrome (`ContextMenu`, number-format, color pickers) flows through
/// this component, every panel inherits edge-safe positioning for free.
///
/// # Trigger buttons
/// The button that opens this popover must stop `pointerdown` propagation so
/// `on_click_outside` does not immediately re-close on the same event:
/// `on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()`.
///
/// # Mount strategy
/// Uses `display:none` on a wrapper rather than `<Show when=>` because
/// `children: Children` is `FnOnce` (called once at mount) and `<Show>`
/// requires its children closure to be `Fn`. Keeping the panel mounted also
/// lets [`use_element_size`] observe it across open/close cycles.
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

    // Reactive measurements: the panel's own box and the viewport. Both update
    // the position the instant either changes, giving us place-then-measure
    // floating behaviour without an imperative Effect.
    let UseElementSizeReturn {
        width: panel_w,
        height: panel_h,
    } = use_element_size(panel_ref);
    let UseWindowSizeReturn {
        width: win_w,
        height: win_h,
    } = use_window_size();

    // Gap kept between the panel and the viewport edge (also the offset from
    // the anchor in `above_anchor` mode, preserving the prior `+ 4px`).
    const MARGIN: f64 = 4.0;

    let panel_style = move || {
        let (x, y) = pos.get();
        let (x, y) = (x as f64, y as f64);
        let (pw, ph) = (panel_w.get(), panel_h.get());
        let (vw, vh) = (win_w.get(), win_h.get());

        // Horizontal: keep the panel within [MARGIN, vw - pw - MARGIN]. The
        // `.max(MARGIN)` floor guarantees the clamp's upper bound never drops
        // below its lower bound (panel wider than the viewport).
        let max_left = (vw - pw - MARGIN).max(MARGIN);
        let left = x.clamp(MARGIN, max_left);

        if above_anchor {
            // Panel grows upward; `bottom` is measured from the viewport
            // bottom. Clamp so the (upper) edge stays on-screen.
            let max_bottom = (vh - ph - MARGIN).max(MARGIN);
            let bottom = (vh - y + MARGIN).clamp(MARGIN, max_bottom);
            format!("left:{left}px;bottom:{bottom}px;")
        } else {
            // Panel grows downward; clamp so the bottom edge stays on-screen.
            let max_top = (vh - ph - MARGIN).max(MARGIN);
            let top = y.clamp(MARGIN, max_top);
            format!("left:{left}px;top:{top}px;")
        }
    };

    view! {
        <div style=move || if open.get() { "" } else { "display:none;" }>
            <div node_ref=panel_ref class=class style=panel_style>
                {children()}
            </div>
        </div>
    }
}
