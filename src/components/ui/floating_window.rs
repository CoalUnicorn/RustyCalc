//! Generic non-modal floating window.
//!
//! A persistent, draggable, resizable panel. It owns the title bar, the close
//! action, the drag and resize interaction, the size, and the position — and
//! nothing else. It imports no domain code: a caller passes a title and its
//! content, and keeps its own state.
//!
//! # Composition
//! Built on [`Popover`] with `dismiss_on_outside_click = false`, so the
//! viewport measurement and position clamping are reused and a click on the
//! surface behind the window never dismisses it. The window's own close
//! action is the only dismissal.
//!
//! # Event boundaries
//! The root stops `keydown`, `pointerdown`, `contextmenu`, and `wheel`
//! propagation, so no window control can reach the workbook input router.
//! Escape closes the window when the event reaches the root and no child has
//! already handled it (a child that handles Escape calls `prevent_default`).
//!
//! # Placement
//! Position is in viewport coordinates, matching [`Popover`]. The size is
//! clamped to the viewport, so a window resize can never push the window
//! off-screen; the title bar offers a Reset action and keyboard-reachable
//! size presets.

use leptos::prelude::*;
use leptos_use::{UseWindowSizeReturn, use_window_size};
use wasm_bindgen::JsCast;

use crate::components::ui::popover::Popover;

/// Keyboard-reachable size presets, in the order they appear in the control.
const SIZE_PRESETS: [(&str, f64, f64); 4] = [
    ("Compact", 480.0, 360.0),
    ("Default", 760.0, 520.0),
    ("Wide", 1000.0, 560.0),
    ("Tall", 760.0, 760.0),
];

/// Smallest usable window. Below this a view cannot lay out at all.
const MIN_WIDTH: f64 = 420.0;
const MIN_HEIGHT: f64 = 320.0;

/// Breathing room kept between the window and the viewport edge. Larger than
/// the [`Popover`] margin so a clamped size still leaves the position clamp
/// room to work.
const MARGIN: f64 = 8.0;

/// A non-modal floating window.
///
/// `open`/`set_open` are the caller's; the caller also owns what "closed"
/// means for its own state. `on_close` runs before the window closes, so a
/// caller can pause work or move focus first.
#[component]
pub fn FloatingWindow(
    open: ReadSignal<bool>,
    set_open: WriteSignal<bool>,
    title: &'static str,
    #[prop(default = (760.0, 520.0))] default_size: (f64, f64),
    #[prop(default = (24.0, 24.0))] default_pos: (f64, f64),
    #[prop(optional, into)] on_close: Option<Callback<()>>,
    children: Children,
) -> impl IntoView {
    let pos = RwSignal::new((default_pos.0 as i32, default_pos.1 as i32));
    let size = RwSignal::new(default_size);
    let shell = NodeRef::<leptos::html::Div>::new();

    Effect::new(move |_| {
        if open.get()
            && let Some(element) = shell.get()
        {
            request_animation_frame(move || {
                if open.try_get_untracked() == Some(true) {
                    let _ = element.focus();
                }
            });
        }
    });

    let UseWindowSizeReturn {
        width: viewport_w,
        height: viewport_h,
    } = use_window_size();

    let close = move || {
        if let Some(on_close) = on_close {
            on_close.run(());
        }
        set_open.set(false);
    };

    // ---- drag ----
    // Pointer offset from the window origin at grab time. Capture keeps the
    // moves flowing when the cursor outruns the grip. Both the grab and the
    // move use client coordinates, so the offset cancels out and no per-move
    // DOM read is needed.
    let drag_offset: StoredValue<Option<(f64, f64)>, LocalStorage> = StoredValue::new_local(None);

    let on_grip_down = move |ev: web_sys::PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        if let Some(el) = ev
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let Some(element) = shell.get_untracked() else {
            return;
        };
        let Some(panel) = element.parent_element() else {
            return;
        };
        let bounds = panel.get_bounding_client_rect();
        let (x, y) = (bounds.x() as i32, bounds.y() as i32);
        pos.set((x, y));
        drag_offset.set_value(Some((
            ev.client_x() as f64 - f64::from(x),
            ev.client_y() as f64 - f64::from(y),
        )));
    };

    let on_grip_move = move |ev: web_sys::PointerEvent| {
        let Some((dx, dy)) = drag_offset.get_value() else {
            return;
        };
        // Capture can be lost without a pointerup (alt-tab, touch interrupt);
        // a buttonless move means the drag already ended.
        if ev.buttons() == 0 {
            drag_offset.set_value(None);
            return;
        }
        let x = (ev.client_x() as f64 - dx).max(0.0);
        let y = (ev.client_y() as f64 - dy).max(0.0);
        pos.set((x as i32, y as i32));
    };

    let on_grip_up = move |ev: web_sys::PointerEvent| {
        if let Some(el) = ev
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.release_pointer_capture(ev.pointer_id());
        }
        drag_offset.set_value(None);
    };

    // ---- resize ----
    // (start_x, start_y, start_w, start_h) at pointer-down. Same pointer
    // capture pattern as the drag; one bottom-right corner handle.
    let resize_grab: StoredValue<Option<(f64, f64, f64, f64)>, LocalStorage> =
        StoredValue::new_local(None);

    let on_handle_down = move |ev: web_sys::PointerEvent| {
        if ev.button() != 0 {
            return;
        }
        ev.prevent_default();
        if let Some(el) = ev
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let Some(element) = shell.get_untracked() else {
            return;
        };
        let bounds = element.get_bounding_client_rect();
        let (w, h) = (bounds.width(), bounds.height());
        resize_grab.set_value(Some((ev.client_x() as f64, ev.client_y() as f64, w, h)));
    };

    let on_handle_move = move |ev: web_sys::PointerEvent| {
        let Some((start_x, start_y, start_w, start_h)) = resize_grab.get_value() else {
            return;
        };
        if ev.buttons() == 0 {
            resize_grab.set_value(None);
            return;
        }
        let w = start_w + (ev.client_x() as f64 - start_x);
        let h = start_h + (ev.client_y() as f64 - start_y);
        size.set((w.max(MIN_WIDTH), h.max(MIN_HEIGHT)));
    };

    let on_handle_up = move |ev: web_sys::PointerEvent| {
        if let Some(el) = ev
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        {
            let _ = el.release_pointer_capture(ev.pointer_id());
        }
        resize_grab.set_value(None);
    };

    // ---- title-bar actions ----
    let on_reset = move |_: web_sys::MouseEvent| {
        pos.set((default_pos.0 as i32, default_pos.1 as i32));
        size.set(default_size);
    };

    let on_preset = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(select) = target.dyn_into::<web_sys::HtmlSelectElement>() else {
            return;
        };
        let Ok(index) = select.value().parse::<usize>() else {
            return;
        };
        if let Some((_, w, h)) = SIZE_PRESETS.get(index) {
            size.set((*w, *h));
        }
    };

    // Buttons must not start a title-bar drag.
    let on_control_down = move |ev: web_sys::PointerEvent| ev.stop_propagation();

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        ev.stop_propagation();
        if ev.key() == "Escape" && !ev.default_prevented() {
            ev.prevent_default();
            close();
        }
    };

    let shell_style = move || {
        let (w, h) = size.get();
        let max_w = (viewport_w.get() - 2.0 * MARGIN).max(1.0);
        let max_h = (viewport_h.get() - 2.0 * MARGIN).max(1.0);
        let w = w.clamp(MIN_WIDTH.min(max_w), max_w);
        let h = h.clamp(MIN_HEIGHT.min(max_h), max_h);
        format!("width:{w}px;height:{h}px;")
    };

    view! {
        <Popover
            open=open
            set_open=set_open
            pos=pos.read_only()
            class="fw-panel"
            dismiss_on_outside_click=false
        >
            <div
                class="fw-shell"
                node_ref=shell
                role="dialog"
                aria-modal="false"
                aria-label=title
                tabindex="-1"
                style=shell_style
                on:keydown=on_keydown
                on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                on:contextmenu=|ev: web_sys::MouseEvent| ev.stop_propagation()
                on:wheel=|ev: web_sys::WheelEvent| ev.stop_propagation()
            >
                <div
                    class="fw-titlebar"
                    on:pointerdown=on_grip_down
                    on:pointermove=on_grip_move
                    on:pointerup=on_grip_up
                    on:pointercancel=on_grip_up
                >
                    <span class="fw-title">{title}</span>
                    <label class="fw-preset-label">
                        "Size"
                        <select
                            class="fw-preset"
                            title="Size preset"
                            prop:value=move || SIZE_PRESETS.iter()
                                .position(|(_, w, h)| (*w, *h) == size.get())
                                .map_or_else(|| "custom".to_owned(), |index| index.to_string())
                            on:pointerdown=on_control_down
                            on:change=on_preset
                        >
                            <option value="custom" disabled>"Custom"</option>
                            {SIZE_PRESETS
                                .iter()
                                .enumerate()
                                .map(|(index, (label, _, _))| {
                                    view! {
                                        <option value=index.to_string()>{*label}</option>
                                    }
                                })
                                .collect_view()}
                        </select>
                    </label>
                    <button
                        class="fw-btn"
                        type="button"
                        title="Reset position and size"
                        on:pointerdown=on_control_down
                        on:click=on_reset
                    >
                        "Reset"
                    </button>
                    <button
                        class="fw-btn fw-close"
                        type="button"
                        title="Close"
                        on:pointerdown=on_control_down
                        on:click=move |_| close()
                    >
                        "\u{2715}"
                    </button>
                </div>
                <div class="fw-body">{children()}</div>
                <div
                    class="fw-resize"
                    title="Resize"
                    on:pointerdown=on_handle_down
                    on:pointermove=on_handle_move
                    on:pointerup=on_handle_up
                    on:pointercancel=on_handle_up
                ></div>
            </div>
        </Popover>
    }
}
