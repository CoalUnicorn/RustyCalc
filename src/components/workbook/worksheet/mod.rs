use leptos::html;
use leptos::prelude::*;
use leptos_use::use_resize_observer;

use crate::components::panels::conditional_formatting::ConditionalFormattingDialog;
use crate::components::panels::link_tooltip::LinkTooltip;
use crate::components::panels::named_ranges::NamedRangesDialog;
use crate::components::workbook::editing::cell_editor::CellEditor;
use crate::input::mouse::{
    CanvasHandle, clear_hover, handle_contextmenu, handle_dblclick, handle_mousedown,
    handle_mousemove, handle_mouseup, handle_wheel, revalidate_hover,
};
use crate::model::AppClipboard;
use crate::state::{DragState, ModelStore, WorkbookState};
use iron_canvas_core::CanvasSize;

mod autofit;
mod overlay_memo;
mod raf_loop;
mod subscribe;

use overlay_memo::reactive_overlay;

pub(super) type ClipboardDraw = StoredValue<Option<AppClipboard>, LocalStorage>;

/// The spreadsheet canvas element.
///
/// Subscribes to `EventBus` signals and the `reactive_overlay` memo so the
/// canvas repaints when model state or drag overlays change. Handles the
/// full mouse interaction set: click-to-select, drag-to-select, autofill
/// handle drag, double-click-to-edit, and wheel scrolling.
#[component]
pub fn Worksheet() -> impl IntoView {
    let grid_ref = NodeRef::<html::Canvas>::new();
    // Scene render handle, provided by `App` so the toolbar and the link
    // tooltip can read committed canvas state too. None until the <canvas>
    // mounts and the container has nonzero CSS dimensions; then constructed
    // exactly once by the lazy-construct block in the rAF loop. Dropped in
    // on_cleanup.
    let canvas_handle: CanvasHandle = expect_context::<CanvasHandle>();
    // Theme-change fence. Set when `events.theme` fires; consumed in the rAF
    // callback below. Defers reading CSS vars to after leptos-use has written
    // the new `data-theme` attribute on `<html>` — reading them synchronously
    // inside the same effect batch as the toggle would race the attribute
    // write and yield stale values.
    let theme_dirty: StoredValue<bool> = StoredValue::new(false);
    on_cleanup(move || {
        canvas_handle.update_value(|slot| *slot = None);
    });
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // ResizeObserver: re-render when the container changes size. Leptos
    // signals don't fire on DOM resize, so we use a ResizeObserver instead
    // (e.g. browser window resize, devtools open/close). Registered further
    // down, once `poke` exists — see the comment there.
    let container_ref = NodeRef::<html::Div>::new();

    let clipboard_draw = expect_context::<ClipboardDraw>();
    let reactive_overlay = reactive_overlay(state, model);

    // `install_raf_loop` runs first so `poke` exists before anything below
    // needs to wake the (now demand-driven, self-pausing) render loop.
    let poke = raf_loop::install_raf_loop(
        grid_ref,
        canvas_handle,
        model,
        state,
        clipboard_draw,
        theme_dirty,
    );

    // Revalidate the idle hover after every commit. The event signals above
    // only *schedule* a paint, so an effect driven by them reads the previous
    // frame; this one runs after the frame that changed the link, the scroll
    // offset, or the sheet, and re-hit-tests the stored pointer position.
    Effect::new(move |_| {
        let _ = state.committed_frame.get();
        revalidate_hover(state, model, canvas_handle);
    });

    // Cleanup is automatic when the component unmounts. Needs `poke`, so it
    // is registered here rather than alongside `container_ref` above.
    {
        let poke = poke.clone();
        let _ = use_resize_observer(container_ref, move |_, _| {
            // Mirror the new dims into the scene session. If the ref hasn't
            // resolved yet, the rAF lazy-construct picks up the current size
            // on its next tick.
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let dpr = window().device_pixel_ratio();
            canvas_handle.update_value(|slot| {
                if let Some(handle) = slot.as_mut() {
                    // Invalid metrics leave the canvas at its last valid size.
                    let _ = handle.resize(grid_el, CanvasSize { w, h }, dpr);
                }
            });
            poke();
        });
    }

    subscribe::install_subscribe_effect(state, theme_dirty, reactive_overlay, poke.clone());

    // Grow rows to fit multi-line / wrapped content on commit. Lives here
    // because it needs the `CanvasHandle` to measure glyphs; watches content
    // events only, so it can't loop on its own `SetRowHeight` (Format) emits.
    autofit::install_autofit_effect(state, canvas_handle, model);

    // Workbook-switch Effect — watching `current_uuid` gives us a deterministic
    // signal that fires once per workbook switch. The model store now holds the
    // new workbook, so the render loop would read it on the next poke; bumping
    // the revision here forces the scene session to re-prepare rather than
    // reuse the outgoing workbook's frame (the rAF loop also rolls the
    // workbook generation on a uuid change).
    {
        let current_uuid = state.current_uuid.read();
        let poke = poke.clone();
        Effect::new(move |_| {
            let _uuid = current_uuid.get();
            state
                .render_revision
                .update(|revision| *revision = revision.wrapping_add(1));
            poke();
        });
    }

    // mousedown: dispatches via the scene hit-test (canvas_handle owns the
    // painted-frame snapshot every event resolves against).
    let on_mousedown = move |ev: web_sys::MouseEvent| {
        handle_mousedown(ev, model, state, canvas_handle);
    };

    // mousemove: expand selection or autofill preview.
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        handle_mousemove(ev, model, state, canvas_handle);
    };

    let on_mouseup = move |ev: web_sys::MouseEvent| {
        handle_mouseup(ev, model, state);
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        handle_dblclick(ev, model, state, canvas_handle);
    };

    // contextmenu: right-click on column/row header.
    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        handle_contextmenu(ev, model, state, canvas_handle);
    };

    // wheel: scroll with delta-magnitude awareness.
    let on_wheel = move |ev: web_sys::WheelEvent| {
        handle_wheel(ev, model, state);
    };

    view! {
        <div
            node_ref=container_ref
            class="ws"
            on:mouseleave=move |_| clear_hover(state)
        >
            <canvas
                node_ref=grid_ref
                role="application"
                aria-label="Spreadsheet grid"
                class=move || {
                    // Drag wins over the idle hover hint: a started resize must
                    // not flicker back to `cell` if the pointer drifts off the
                    // 4-px hot-zone mid-drag.
                    let extra = match state.drag.get() {
                        DragState::ResizingCol { .. } => "resize-col",
                        DragState::ResizingRow { .. } => "resize-row",
                        DragState::Idle
                        | DragState::Selecting
                        | DragState::Extending { .. }
                        | DragState::Pointing { .. }
                        | DragState::DraggingFormulaRef { .. } => state.hover_cursor.get().class(),
                    };
                    if extra.is_empty() {
                        "ws-canvas ws-grid".to_string()
                    } else {
                        format!("ws-canvas ws-grid {extra}")
                    }
                }
                tabindex="-1"
                on:mousedown=on_mousedown
                on:mousemove=on_mousemove
                on:mouseup=on_mouseup
                on:dblclick=on_dblclick
                on:wheel=on_wheel
                on:contextmenu=on_contextmenu
            />
            <CellEditor />
            <LinkTooltip grid_ref=grid_ref />
            <NamedRangesDialog />
            <ConditionalFormattingDialog />
        </div>
    }
}
