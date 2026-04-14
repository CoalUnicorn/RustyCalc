use leptos::html;
use leptos::prelude::*;
use leptos_use::{use_raf_fn, use_resize_observer};
use web_sys::HtmlCanvasElement;

use crate::app_state::AppState;
use crate::canvas::*;
use crate::components::cell_editor::CellEditor;
use crate::coord::{CellArea, SheetArea};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::mouse::*;
use crate::model::AppClipboard;
use crate::state::{DragState, ModelStore, WorkbookState};

/// The spreadsheet canvas element.
///
/// Subscribes to `EventBus` signals and the `reactive_overlay` memo so the
/// canvas repaints when model state or drag overlays change.
/// Handles the full mouse interaction set: click-to-select, drag-to-select,
/// autofill handle drag, double-click-to-edit, and wheel scrolling.
#[component]
pub fn Worksheet() -> impl IntoView {
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let state = expect_context::<WorkbookState>();
    let app = expect_context::<AppState>();
    let model = expect_context::<ModelStore>();

    // ResizeObserver: re-render when the container changes size
    // Leptos signals don't fire on DOM resize, so we use a ResizeObserver
    // that bumps the redraw counter whenever the worksheet div is resized
    // (e.g. browser window resize, devtools open/close).
    // Cleanup is automatic when the component unmounts.
    let container_ref = NodeRef::<html::Div>::new();
    let _ = use_resize_observer(container_ref, move |_, _| {
        state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
    });

    // Re-render canvas every time visual events occur (content, format, navigation, structure).
    let clipboard_draw = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();

    // Memo for canvas theme - cached until theme changes.
    let canvas_theme = Memo::new(move |_| app.get_theme().canvas_theme());

    // Memo for the reactive overlay components (autofill extend target and
    // point-mode range). These must live in a memo, not be read directly in
    // the subscription Effect: if the Effect subscribed to drag/point_range
    // directly, set_drag(Selecting) in on_mousedown would cause an extra
    // Effect run (and an extra render) before the navigation event fires.
    //
    // The memo's PartialEq gate also suppresses spurious renders: Selecting
    // and Idle both map to extend_to=None, so switching between them doesn't
    // change the memo output and doesn't re-render.
    //
    // The clipboard is NOT in this memo because it lives in a StoredValue
    // (non-reactive). It is read fresh in the rAF callback each render so it
    // never goes stale (the original marching-ants bug).
    let reactive_overlay = Memo::new(move |_| {
        let extend_to = if let DragState::Extending { to_row, to_col } = state.drag.get() {
            Some(AutofillTarget {
                row: to_row,
                col: to_col,
            })
        } else {
            None
        };

        let point_range = if let DragState::Pointing { range, .. } = state.drag.get() {
            Some(range)
        } else {
            None
        };

        (extend_to, point_range)
    });

    // Flag: set by the reactive subscription Effect below, cleared by the
    // rAF render loop. Starts true so the first animation frame draws the
    // initial state without waiting for an event.
    let render_needed = RwSignal::new(true);

    // Tracks which render path is needed; written by the subscription Effect
    // below, available to the rAF closure for future per-mode dispatch.
    let render_mode = RwSignal::new(CanvasRenderMode::Full);

    // Reactive subscription Effect - tracks events and overlay changes.
    // Does NOT render. Only sets the flag so the rAF loop below can do the
    // draw on the next animation frame.
    //
    // Decoupling subscription from rendering is the key to smooth navigation:
    // holding an arrow key fires ~30 keydown events per second, each emitting
    // a NavigationEvent. Without rAF coalescing every event would trigger a
    // synchronous canvas render. With this split, all events in a single
    // 16 ms frame coalesce into one draw call.
    //
    // Per-category subscription: reads directly from EventBus signals.
    // Each category signal is replaced (not appended) on every emit, so
    // reading any non-empty signal means a new action just happened.
    // The Effect returns the current overlay state so the next run can
    // detect overlay-only changes (autofill preview, point-mode range)
    // without needing a fake ContentEvent::GenericChange from request_redraw().
    Effect::new(
        move |prev: Option<(Option<AutofillTarget>, Option<CellArea>)>| {
            let has_content = !state.events.content.get().is_empty();
            let has_structure = !state.events.structure.get().is_empty();
            let has_format = !state.events.format.get().is_empty();
            let has_nav = !state.events.navigation.get().is_empty();
            let has_theme = !state.events.theme.get().is_empty();
            let overlay = reactive_overlay.get();
            let overlay_changed = prev.is_some_and(|p| p != overlay);

            let mode = if has_content || has_structure || has_theme {
                CanvasRenderMode::Full
            } else if has_format {
                CanvasRenderMode::FormatOnly
            } else if has_nav {
                CanvasRenderMode::ViewportUpdate
            } else if overlay_changed {
                CanvasRenderMode::Overlay
            } else {
                return overlay;
            };

            render_mode.set(mode);
            render_needed.set(true);
            overlay
        },
    );

    // rAF render loop - fires on every animation frame (~60 fps).
    // Renders only when render_needed is true; otherwise returns immediately
    // (single untracked signal read + branch).
    let _ = use_raf_fn(move |_| {
        if !render_needed.get_untracked() {
            return;
        }
        render_needed.set(false);

        let Some(canvas) = canvas_ref.get_untracked() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        // Sync canvas dimensions into the model so scroll/autofill knows the
        // visible viewport size. Dimension check is cheap; CanvasRenderer::new
        // only reallocates the backing store when dimensions actually changed.
        let canvas_w = canvas_el.client_width() as f64;
        let canvas_h = canvas_el.client_height() as f64;
        model.update_value(|m| {
            m.set_window_width(canvas_w);
            m.set_window_height(canvas_h);
        });
        let (extend_to, point_range) = reactive_overlay.get_untracked();
        let clipboard = clipboard_draw.with_value(|opt| {
            opt.as_ref().map(|acb| SheetArea {
                sheet: acb.sheet,
                area: acb.range,
            })
        });
        let overlays = RenderOverlays {
            extend_to,
            clipboard,
            point_range,
        };
        model.with_value(|m| {
            let mut renderer = CanvasRenderer::new(&canvas_el, *canvas_theme.get_untracked());
            renderer.render(m, &overlays);
        });
        // Record render-done timestamp for the perf panel.
        if app.perf.commit_start.get_untracked().is_some() {
            app.perf.render_done.set(Some(crate::perf::now()));
        }
    });

    // mousedown: dispatches to one of the six named handlers below.
    let on_mousedown = move |ev: web_sys::MouseEvent| {
        handle_mousedown(ev, model, state);
    };

    // mousemove: expand selection or autofill preview
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        handle_mousemove(ev, model, state);
    };

    let on_mouseup = move |ev: web_sys::MouseEvent| {
        handle_mouseup(ev, model, state);
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        handle_dblclick(ev, model, state);
    };

    // contextmenu: right-click on column/row header
    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        handle_contextmenu(ev, model, state);
    };

    // wheel: scroll with delta-magnitude awareness
    let on_wheel = move |ev: web_sys::WheelEvent| {
        handle_wheel(ev, model, state);
    };

    view! {
        <div node_ref=container_ref class="ws">
            <canvas
                node_ref=canvas_ref
                role="application"
                aria-label="Spreadsheet grid"
                class=move || {
                    match state.drag.get() {
                        DragState::ResizingCol { .. } => "ws-canvas resize-col",
                        DragState::ResizingRow { .. } => "ws-canvas resize-row",
                        DragState::Idle
                        | DragState::Selecting
                        | DragState::Extending { .. }
                        | DragState::Pointing { .. } => "ws-canvas",
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
        </div>
    }
}
