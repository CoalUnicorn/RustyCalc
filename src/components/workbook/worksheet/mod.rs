use leptos::html;
use leptos::prelude::*;
use leptos_use::use_resize_observer;
use std::rc::Rc;

use crate::app_state::AppState;
use crate::components::workbook::editing::cell_editor::CellEditor;
use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::input::mouse::{
    CanvasHandle, handle_contextmenu, handle_dblclick, handle_mousedown, handle_mousemove,
    handle_mouseup, handle_wheel,
};
use crate::model::AppClipboard;
use crate::state::{DragState, ModelStore, WorkbookState};

mod adapter;
#[cfg(feature = "dev-tools")]
mod dev_tools_effects;
mod overlay_memo;
mod raf_loop;
mod subscribe;

use adapter::WorksheetModelAdapter;
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
    let overlay_ref = NodeRef::<html::Canvas>::new();
    // IronCanvas orchestrator handle. None until both <canvas> elements mount
    // and the container has nonzero CSS dimensions; then constructed exactly
    // once by the lazy-construct block in the rAF loop. Disposed in on_cleanup.
    let canvas_handle: CanvasHandle = StoredValue::new_local(None);
    // Theme-change fence. Set when `events.theme` fires; consumed in the rAF
    // callback below. Defers `setThemeFromElement` to after leptos-use has
    // written the new `data-theme` attribute on `<html>` — reading CSS vars
    // synchronously inside the same effect batch as the toggle would race the
    // attribute write and yield stale values.
    let theme_dirty: StoredValue<bool> = StoredValue::new(false);
    // Expose the handle to descendant components (e.g. FormulaTextArea needs
    // `cell_rect` to position the in-cell editor against the painted frame).
    provide_context(canvas_handle);
    on_cleanup(move || {
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.take() {
                ic.dispose();
            }
        });
    });
    let state = expect_context::<WorkbookState>();
    let app = use_context::<AppState>();
    let model = expect_context::<ModelStore>();

    // ResizeObserver: re-render when the container changes size.
    // Leptos signals don't fire on DOM resize, so we use a ResizeObserver
    // that bumps the redraw counter whenever the worksheet div is resized
    // (e.g. browser window resize, devtools open/close).
    // Cleanup is automatic when the component unmounts.
    let container_ref = NodeRef::<html::Div>::new();
    let _ = use_resize_observer(container_ref, move |_, _| {
        // During playback the orchestrator + canvas backing stores are
        // pinned to the recording's dimensions; a live container resize
        // (window resize, devtools) would otherwise clobber them and skew
        // the replay.
        #[cfg(feature = "dev-tools")]
        if app.playback_loaded.get_untracked() {
            return;
        }

        state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));

        // Mirror the new dims into the orchestrator. Both canvases share CSS
        // dims, so reading from grid_ref is sufficient. If the ref hasn't
        // resolved yet, the rAF lazy-construct will pick up the current size
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
            if let Some(ic) = slot.as_mut() {
                ic.resize(w, h, dpr);
                ic.request_repaint();
            }
        });
    });

    let clipboard_draw = expect_context::<ClipboardDraw>();
    let reactive_overlay = reactive_overlay(state);

    // Flag: set by the reactive subscription Effect, cleared by the rAF
    // render loop. Starts true so the first animation frame draws the
    // initial state without waiting for an event.
    let render_needed = RwSignal::new(true);

    subscribe::install_subscribe_effect(
        state,
        canvas_handle,
        theme_dirty,
        reactive_overlay,
        clipboard_draw,
        render_needed,
    );

    // Workbook-switch Effect — watching `current_uuid` gives us a deterministic
    // signal that fires once per workbook switch. Without a set_model call,
    // the orchestrator keeps last_frame from the old workbook (stale pane
    // geometry, stale sheet ID), and paint_if_dirty never drops it for a
    // Fresh rebuild. `set_model` is idempotent-safe — re-pushing the same
    // adapter triggers a full repaint.
    {
        let current_uuid = state.current_uuid.read();
        Effect::new(move |_| {
            let _uuid = current_uuid.get();
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.set_model(Rc::new(WorksheetModelAdapter { store: model }));
                }
            });
        });
    }

    #[cfg(feature = "dev-tools")]
    {
        dev_tools_effects::install_recording_effect(state, app, canvas_handle);
        dev_tools_effects::install_playback_effect(state, app, canvas_handle, render_needed);
        dev_tools_effects::install_export_effect(state, app, canvas_handle);
    }

    raf_loop::install_raf_loop(
        grid_ref,
        overlay_ref,
        canvas_handle,
        model,
        reactive_overlay,
        clipboard_draw,
        theme_dirty,
        render_needed,
        app,
    );

    // mousedown: dispatches via IronCanvas::hit_test (canvas_handle owns the
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

    // Allow scrolling the container while a recording is loaded — the
    // recording may be larger than the current viewport. Reverts to the
    // CSS default (`hidden` for `.ws`) when playback exits.
    #[cfg(feature = "dev-tools")]
    let container_overflow = move || {
        if app.playback_loaded.get() {
            "auto"
        } else {
            ""
        }
    };
    #[cfg(not(feature = "dev-tools"))]
    let container_overflow = move || "";

    view! {
        <div node_ref=container_ref class="ws" style:overflow=container_overflow>
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
            <canvas
                node_ref=overlay_ref
                class="ws-canvas ws-overlay"
                aria-hidden="true"
            />
            <CellEditor />
        </div>
    }
}
