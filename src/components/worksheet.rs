use iron_canvas_core::types::coord::AutofillTarget;
use ironcalc_base::types::{CellType, Style};
use leptos::html;
use leptos::prelude::*;
use leptos_use::{use_raf_fn, use_resize_observer};
use std::rc::Rc;
use web_sys::HtmlCanvasElement;

#[cfg(feature = "dev-tools")]
use crate::app_state::{AppState, RecordingCmd};
use crate::components::cell_editor::CellEditor;
use crate::coord::ActiveRef;
use crate::coord::{CellArea, SheetRange};
use crate::events::{ContentEvent, SpreadsheetEvent};
use crate::input::mouse::*;
use crate::model::AppClipboard;
#[cfg(feature = "dev-tools")]
use crate::state::StatusMessage;
use crate::state::{DragState, ModelStore, WorkbookState};
use iron_canvas_core::*;
use iron_canvas_web::IronCanvas;

/// Bridges `ModelStore` (a Leptos `StoredValue` holding `UserModel<'static>`)
/// to `iron_canvas::CanvasModel`. Each trait method `with_value`-borrows the
/// current `UserModel` and dispatches through its existing `CanvasModel`
/// impl. The handle (`ModelStore`) is `Copy`, so the adapter is freely
/// `'static` and the wrapping `Rc<dyn CanvasModel>` is stable across the
/// component's lifetime — workbook switches that replace the inner
/// `UserModel` are picked up automatically on the next render-time read.
struct WorksheetModelAdapter {
    store: ModelStore,
}

impl CanvasModel for WorksheetModelAdapter {
    fn get_selected_sheet(&self) -> u32 {
        self.store.with_value(CanvasModel::get_selected_sheet)
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        self.store.with_value(CanvasModel::get_selected_view)
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| CanvasModel::get_frozen_rows_count(m, sheet))
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.store
            .with_value(|m| CanvasModel::get_frozen_columns_count(m, sheet))
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.store
            .with_value(|m| CanvasModel::get_row_height(m, sheet, row))
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.store
            .with_value(|m| CanvasModel::get_column_width(m, sheet, column))
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.store
            .with_value(|m| CanvasModel::get_show_grid_lines(m, sheet))
    }
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Option<Style> {
        self.store
            .with_value(|m| CanvasModel::get_cell_style(m, sheet, row, column))
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Option<CellType> {
        self.store
            .with_value(|m| CanvasModel::get_cell_type(m, sheet, row, column))
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Option<String> {
        self.store
            .with_value(|m| CanvasModel::get_formatted_cell_value(m, sheet, row, column))
    }
}

/// The spreadsheet canvas element.
///
/// Subscribes to `EventBus` signals and the `reactive_overlay` memo so the
/// canvas repaints when model state or drag overlays change.
/// Handles the full mouse interaction set: click-to-select, drag-to-select,
/// autofill handle drag, double-click-to-edit, and wheel scrolling.
#[component]
pub fn Worksheet() -> impl IntoView {
    let grid_ref = NodeRef::<html::Canvas>::new();
    let overlay_ref = NodeRef::<html::Canvas>::new();
    // IronCanvas orchestrator handle. None until both <canvas> elements mount
    // and the container has nonzero CSS dimensions; then constructed exactly
    // once by the lazy-construct block in the rAF loop. Disposed in on_cleanup.
    let canvas_handle: StoredValue<Option<IronCanvas>, LocalStorage> = StoredValue::new_local(None);
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
    #[cfg(feature = "dev-tools")]
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

        // Mirror the new dims into the orchestrator. Both canvases share
        // CSS dims, so reading from grid_ref is sufficient. If the ref
        // hasn't resolved yet, the rAF lazy-construct will pick up the
        // current size on its next tick.
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
                ic.requestRepaint();
            }
        });
    });

    // Re-render canvas every time visual events occur (content, format, navigation, structure).
    let clipboard_draw = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();

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

        // Reading editing_cell here subscribes the memo to it. Since FormulaRef
        // derives PartialEq, the memo's PartialEq gate suppresses re-renders when
        // refs don't change (e.g. text changed but no new refs produced).
        let editing_cell = state.editing_cell.get();
        let mut formula_refs: Vec<ActiveRef> = editing_cell
            .as_ref()
            .map(|e| e.formula_analysis.refs().to_vec())
            .unwrap_or_default();

        // Live drag ghost: while `DraggingFormulaRef` is active, mousemove
        // publishes a `RefOverride`; we patch the matching ref's
        // `sheet_area` so the painted outline follows the cursor without
        // touching the formula text. Bounds-checked: if the formula was
        // re-analyzed mid-drag and refs shrank, the patch is silently
        // skipped.
        if let Some(o) = state.dragged_ref_override.get() {
            if let Some(r) = formula_refs.get_mut(o.idx) {
                r.sheet_area = o.range;
            }
        }

        // Index of the ref whose token span contains the caret — drives the
        // renderer's "active" emphasis. Same inclusive predicate as
        // `refs_at_cursor`; first match wins (token-stream order).
        let active_ref: Option<usize> = editing_cell.as_ref().and_then(|e| {
            let cursor = e.cursor;
            e.formula_analysis
                .refs()
                .iter()
                .position(|r| cursor >= r.span.start && cursor <= r.span.end)
        });

        // Point-mode range for overlay painting. RefNode stores relative deltas,
        // so resolution needs the editing cell's address as anchor.
        let point_range = match (state.drag.get(), editing_cell.as_ref()) {
            (DragState::Pointing { ref_node, .. }, Some(e)) => Some(ref_node.area(&e.address).area),
            _ => None,
        };

        (extend_to, point_range, formula_refs, active_ref)
    });

    // Flag: set by the reactive subscription Effect below, cleared by the
    // rAF render loop. Starts true so the first animation frame draws the
    // initial state without waiting for an event.
    let render_needed = RwSignal::new(true);

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
        move |prev: Option<(
            Option<AutofillTarget>,
            Option<CellArea>,
            Vec<ActiveRef>,
            Option<usize>,
        )>| {
            let has_content = !state.events.content.get().is_empty();
            let has_structure = !state.events.structure.get().is_empty();
            let has_format = !state.events.format.get().is_empty();
            let has_nav = !state.events.navigation.get().is_empty();
            let has_theme = !state.events.theme.get().is_empty();
            let overlay = reactive_overlay.get();
            let overlay_changed = prev.is_some_and(|p| p != overlay);

            if !(has_content
                || has_structure
                || has_format
                || has_nav
                || has_theme
                || overlay_changed)
            {
                return overlay;
            }
            render_needed.set(true);

            // Push the same state into the IronCanvas orchestrator. Each
            // setter value-compares, so redundant pushes (e.g. format-only
            // events not touching theme) flip dirty only on the layers that
            // actually need it. requestRepaint() at the end is the safety
            // net that ensures content/format/structure events still fan
            // out to both layers even when no value changed locally.
            let (extend_to, point_range, formula_refs, active_ref) = overlay.clone();
            let clipboard = clipboard_draw.with_value(|opt| {
                opt.as_ref().map(|acb| SheetRange {
                    sheet: acb.sheet,
                    area: acb.range,
                })
            });
            let overlays = RenderOverlays {
                extend_to,
                clipboard: clipboard.map(Into::into),
                point_range: point_range.map(Into::into),
                formula_refs: formula_refs.into_iter().map(Into::into).collect(),
                active_ref,
            };
            if has_theme {
                theme_dirty.set_value(true);
            }
            canvas_handle.update_value(|slot| {
                if let Some(ic) = slot.as_mut() {
                    ic.set_overlays(overlays);
                    // Format events include row/col resize (LayoutChanged) —
                    // those mutate slot pixel geometry and must drop
                    // last_frame, so they route through requestRepaint with
                    // structure. Content edits go through markContentDirty
                    // for the SlotsReuse fast path; nav co-firing (commit-
                    // Enter) needs an explicit overlay raise because
                    // markContentDirty leaves the overlay bit untouched.
                    if has_structure || has_format {
                        ic.requestRepaint();
                    } else if has_content {
                        ic.markContentDirty();
                        if has_nav {
                            ic.request_overlay_repaint();
                        }
                    } else if has_nav {
                        ic.request_overlay_repaint();
                    }
                }
            });

            overlay
        },
    );

    // Recording dispatch Effect — peer to the reactive Effect above. Drains
    // `app.recording_cmd` (one-shot Start/Stop from PerfPanel) and forwards
    // to the iron-canvas orchestrator. Gated by `recorder` feature because
    // `IronCanvas::startRecording` / `stopRecording` only exist when the
    // upstream `iron-canvas-web/recorder` feature is enabled.
    //
    // `set(None)` at the end re-fires this same Effect with `cmd == None`,
    // which short-circuits via the `let-else`. No infinite loop.
    #[cfg(feature = "dev-tools")]
    Effect::new(move |_| {
        let Some(cmd) = app.recording_cmd.get() else {
            return;
        };
        canvas_handle.update_value(|slot| {
            let Some(ic) = slot.as_mut() else {
                state
                    .status
                    .set(Some(StatusMessage::Error("canvas not ready".into())));
                return;
            };
            match cmd {
                RecordingCmd::Start => match ic.startRecording() {
                    Ok(()) => app.recording_active.set(true),
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "startRecording failed: {e:?}"
                    )))),
                },
                RecordingCmd::Stop => {
                    // Engine `stopRecording` clears its `recording` state before
                    // it can fail at `serialize()`; reset the UI flag eagerly so
                    // a serialize-Err doesn't wedge the button in "Stop".
                    app.recording_active.set(false);
                    match ic.stopRecording() {
                        Ok(arr) => {
                            let bytes = arr.to_vec();
                            let ts = js_sys::Date::new_0()
                                .to_iso_string()
                                .as_string()
                                .and_then(|s| s.split('.').next().map(str::to_owned))
                                .map(|s| s.replace(':', "-"))
                                .unwrap_or_else(|| "now".into());
                            let filename = format!("recording-{ts}.icr");
                            crate::input::xlsx_io::trigger_download(
                                &bytes,
                                &filename,
                                Some("application/octet-stream"),
                            );
                        }
                        Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                            "stopRecording failed: {e:?}"
                        )))),
                    }
                }
            }
        });
        app.recording_cmd.set(None);
    });

    // rAF render loop - fires on every animation frame (~60 fps).
    // Renders only when render_needed is true; otherwise returns immediately
    // (single untracked signal read + branch).
    let _ = use_raf_fn(move |_| {
        // Lazy IronCanvas construction. Runs every frame until both refs are
        // Some AND container dims > 0, then becomes a no-op via slot.is_some()
        // short-circuit. Handles the zero-size-container edge case (refs
        // resolved but layout pass hasn't measured yet) without extra
        // ResizeObserver plumbing.
        canvas_handle.update_value(|slot| {
            if slot.is_some() {
                return;
            }
            let Some(grid_el) = grid_ref.get_untracked() else {
                return;
            };
            let Some(overlay_el) = overlay_ref.get_untracked() else {
                return;
            };
            let w = grid_el.client_width() as f64;
            let h = grid_el.client_height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let dpr = window().device_pixel_ratio();
            match IronCanvas::create(grid_el, overlay_el) {
                Ok(mut ic) => {
                    ic.resize(w, h, dpr);
                    // Initial state push: sync current Worksheet state to
                    // the freshly-constructed orchestrator so Task 5's
                    // drop-in swap inherits a correct first frame.
                    // Subsequent pushes are driven by the reactive Effects
                    // below.
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.setThemeFromElement(&el);
                    }
                    ic.set_model(Rc::new(WorksheetModelAdapter { store: model }));
                    let (extend_to, point_range, formula_refs, active_ref) =
                        reactive_overlay.get_untracked();
                    let clipboard = clipboard_draw.with_value(|opt| {
                        opt.as_ref().map(|acb| SheetRange {
                            sheet: acb.sheet,
                            area: acb.range,
                        })
                    });
                    ic.set_overlays(RenderOverlays {
                        extend_to,
                        clipboard: clipboard.map(Into::into),
                        point_range: point_range.map(Into::into),
                        formula_refs: formula_refs.into_iter().map(Into::into).collect(),
                        active_ref,
                    });
                    *slot = Some(ic);
                }
                Err(e) => web_sys::console::error_1(&e),
            }
        });

        if !render_needed.get_untracked() {
            return;
        }
        render_needed.set(false);

        let Some(canvas) = grid_ref.get_untracked() else {
            return;
        };
        let canvas_el: HtmlCanvasElement = canvas;
        // Sync canvas dimensions into the model so scroll/autofill knows the
        // visible viewport size.
        let canvas_w = canvas_el.client_width() as f64;
        let canvas_h = canvas_el.client_height() as f64;
        model.update_value(|m| {
            m.set_window_width(canvas_w);
            m.set_window_height(canvas_h);
        });
        // Renderer debug
        web_sys::console::time_with_label("render");
        #[cfg(feature = "dev-tools")]
        let paint_t0 = crate::perf::now();
        canvas_handle.update_value(|slot| {
            if let Some(ic) = slot.as_mut() {
                if theme_dirty.get_value() {
                    #[cfg(target_arch = "wasm32")]
                    if let Some(el) = window().document().and_then(|d| d.document_element()) {
                        ic.setThemeFromElement(&el);
                    }
                    theme_dirty.set_value(false);
                }
                ic.paintIfDirty();
            }
        });
        // Renderer debug
        web_sys::console::time_end_with_label("render");

        // Record paint duration for the PerfPanel. Skipped until the first
        // cell commit has happened so the panel stays on its placeholder
        // ("commit a cell to measure") and we don't spam the signal on
        // every scroll / resize / overlay tick.
        #[cfg(feature = "dev-tools")]
        if app.perf.commit_start.get_untracked().is_some() {
            app.perf.render_ms.set(Some(crate::perf::now() - paint_t0));
        }
    });

    // mousedown: dispatches via IronCanvas::hit_test (canvas_handle owns the
    // painted-frame snapshot every event resolves against).
    let on_mousedown = move |ev: web_sys::MouseEvent| {
        handle_mousedown(ev, model, state, canvas_handle);
    };

    // mousemove: expand selection or autofill preview
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        handle_mousemove(ev, model, state, canvas_handle);
    };

    let on_mouseup = move |ev: web_sys::MouseEvent| {
        handle_mouseup(ev, model, state);
    };

    let on_dblclick = move |ev: web_sys::MouseEvent| {
        handle_dblclick(ev, model, state, canvas_handle);
    };

    // contextmenu: right-click on column/row header
    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        handle_contextmenu(ev, model, state, canvas_handle);
    };

    // wheel: scroll with delta-magnitude awareness
    let on_wheel = move |ev: web_sys::WheelEvent| {
        handle_wheel(ev, model, state);
    };

    view! {
        <div node_ref=container_ref class="ws">
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
