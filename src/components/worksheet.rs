use ironcalc_base::expressions::types::Area;

use leptos::html;
use leptos::prelude::*;
use leptos_use::{use_raf_fn, use_resize_observer};
use web_sys::HtmlCanvasElement;

use crate::canvas::*;
use crate::components::cell_editor::CellEditor;
use crate::input::formula_input::*;
use crate::model::{AppClipboard, ArrowKey, CellAddress, FrontendModel, PageDir};

use crate::events::{
    ContentEvent, DragState, FormatEvent, HeaderContextMenu, NavigationEvent, SpreadsheetEvent,
};
use crate::state::{ContextMenuState, EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::util::warn_if_err;

/// Pixel tolerance for column/row resize hit-test in the header area.
const HIT_ZONE: f64 = 4.0;

/// The spreadsheet canvas element.
///
/// Subscribes to `WorkbookState.redraw` so the canvas repaints whenever
/// any model mutation calls `WorkbookState::request_redraw()`.
/// Handles the full mouse interaction set: click-to-select, drag-to-select,
/// autofill handle drag, double-click-to-edit, and wheel scrolling.
#[component]
pub fn Worksheet() -> impl IntoView {
    let canvas_ref = NodeRef::<html::Canvas>::new();
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    // ResizeObserver: re-render when the container changes size
    // Leptos signals don't fire on DOM resize, so we use a ResizeObserver
    // that bumps the redraw counter whenever the worksheet div is resized
    // (e.g. browser window resize, devtools open/close).
    // Cleanup is automatic when the component unmounts.
    let container_ref = NodeRef::<html::Div>::new();
    let _ = use_resize_observer(container_ref, move |_, _| {
        state.request_redraw();
    });

    // Re-render canvas every time visual events occur (content, format, navigation, structure).
    let clipboard_draw = expect_context::<StoredValue<Option<AppClipboard>, LocalStorage>>();

    // Memo for canvas theme - cached until theme changes.
    let canvas_theme = Memo::new(move |_| state.get_theme().canvas_theme());

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
    Effect::new(move |_| {
        let has_content = !state.events.content.get().is_empty();
        let has_structure = !state.events.structure.get().is_empty();
        let has_format = !state.events.format.get().is_empty();
        let has_nav = !state.events.navigation.get().is_empty();
        let has_theme = !state.events.theme.get().is_empty();
        let _overlay = reactive_overlay.get();

        let mode = if has_content || has_structure || has_theme {
            CanvasRenderMode::Full
        } else if has_format {
            CanvasRenderMode::FormatOnly
        } else if has_nav {
            CanvasRenderMode::ViewportUpdate
        } else {
            // Mode event only — no canvas repaint needed.
            return;
        };

        render_mode.set(mode);
        render_needed.set(true);
    });

    // rAF render loop - fires on every animation frame (~60 fps).
    // Renders only when render_needed is true; otherwise returns immediately
    // (~1 µs overhead when idle - a single untracked signal read + branch).
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
            opt.as_ref().map(|acb| {
                let (r1, c1, r2, c2) = acb.range;
                ClipboardRange {
                    sheet: acb.sheet,
                    r1,
                    c1,
                    r2,
                    c2,
                }
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
        if state.perf.commit_start.get_untracked().is_some() {
            state.perf.render_done.set(Some(crate::perf::now()));
        }
    });

    // mousedown: dispatches to one of the six named handlers below.
    let on_mousedown = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        if y < HEADER_ROW_HEIGHT
            && x > HEADER_COL_WIDTH
            && try_begin_col_resize(&ev, x, model, state)
        {
            return;
        }
        if x < HEADER_COL_WIDTH
            && y > HEADER_ROW_HEIGHT
            && try_begin_row_resize(&ev, y, model, state)
        {
            return;
        }
        if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
            handle_corner_click(model, state);
            return;
        }
        if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
            handle_col_header_click(&ev, x, model, state);
            return;
        }
        if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
            handle_row_header_click(&ev, y, model, state);
            return;
        }
        handle_cell_click(&ev, x, y, model, state);
    };

    // mousemove: expand selection or autofill preview
    let on_mousemove = move |ev: web_sys::MouseEvent| {
        // If no button is held the drag ended outside the canvas (mouseup was
        // missed). Reset all drag state so the next interaction starts clean.
        if ev.buttons() == 0 {
            state.drag.set(DragState::Idle);
            return;
        }
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        // Resize drags
        match state.drag.get_untracked() {
            DragState::ResizingCol { col, x: last_x } => {
                let delta = x - last_x;
                model.update_value(|m| {
                    let sheet = m.active_cell().sheet;
                    let current_w = m.get_column_width(sheet, col).unwrap_or(DEFAULT_COL_WIDTH);
                    let new_w = (current_w + delta).max(5.0);
                    warn_if_err(
                        m.set_columns_width(sheet, col, col, new_w),
                        "set_columns_width",
                    );
                });
                state.drag.set(DragState::ResizingCol { col, x });
                let sheet = model.with_value(|m| m.active_cell().sheet);
                state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                    sheet,
                    col: Some(col),
                    row: None,
                }));
                ev.prevent_default();
                return;
            }
            DragState::ResizingRow { row, y: last_y } => {
                let delta = y - last_y;
                model.update_value(|m| {
                    let sheet = m.active_cell().sheet;
                    let current_h = m.get_row_height(sheet, row).unwrap_or(DEFAULT_ROW_HEIGHT);
                    let new_h = (current_h + delta).max(3.0);
                    warn_if_err(m.set_rows_height(sheet, row, row, new_h), "set_rows_height");
                });
                state.drag.set(DragState::ResizingRow { row, y });
                let sheet = model.with_value(|m| m.active_cell().sheet);
                state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                    sheet,
                    col: None,
                    row: Some(row),
                }));
                ev.prevent_default();
                return;
            }
            DragState::Idle
            | DragState::Selecting
            | DragState::Extending { .. }
            | DragState::Pointing { .. } => {}
        }

        if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
            return;
        }

        let (row, col) = model.with_value(|m| {
            let view = m.get_selected_view();
            let sheet = view.sheet;
            let fg = frozen_geometry(m, sheet);
            (
                pixel_to_row(m, sheet, view.top_row, y, &fg),
                pixel_to_col(m, sheet, view.left_column, x, &fg),
            )
        });

        match state.drag.get_untracked() {
            DragState::Extending { .. } => {
                // Update autofill preview target.
                state.drag.set(DragState::Extending {
                    to_row: row,
                    to_col: col,
                });
                state.request_redraw();
            }
            DragState::Pointing {
                range: pr,
                ref_span,
            } => {
                // Extend the point-mode range to the hovered cell.
                let sheet = model.with_value(|m| m.active_cell().sheet);
                let ref_str = range_ref_str(pr.r1, pr.c1, row, col, sheet, sheet, "");
                let cursor = ref_span.1;
                let new_state = state
                    .editing_cell
                    .get_untracked()
                    .map(|edit| splice_ref(&edit.text, cursor, &ref_str, Some(ref_span)));
                if let Some((new_text, new_start, new_end)) = new_state {
                    state.editing_cell.update(|c| {
                        if let Some(e) = c {
                            e.text = new_text;
                        }
                    });
                    state.drag.set(DragState::Pointing {
                        range: SheetRect {
                            r1: pr.r1,
                            c1: pr.c1,
                            r2: row,
                            c2: col,
                        },
                        ref_span: (new_start, new_end),
                    });
                    state.request_redraw();
                }
            }
            DragState::Selecting => {
                // pixel_to_col/row start scanning from left_column/top_row, so
                // they can never return a value smaller than the viewport origin.
                // When the pointer is at the leftmost/topmost visible data cell
                // and the viewport is scrolled, nudge the target one step past
                // the edge so on_area_selecting scrolls the viewport left/up.
                let (eff_row, eff_col) = model.with_value(|m| {
                    let view = m.get_selected_view();
                    let ec = if col == view.left_column && view.left_column > 1 {
                        col - 1
                    } else {
                        col
                    };
                    let er = if row == view.top_row && view.top_row > 1 {
                        row - 1
                    } else {
                        row
                    };
                    (er, ec)
                });
                model.update_value(|m| {
                    m.nav_extend_selection(eff_row, eff_col);
                });
                state.request_redraw();
            }
            DragState::Idle | DragState::ResizingCol { .. } | DragState::ResizingRow { .. } => {}
        }
    };

    // mouseup: commit autofill or end selection drag
    let on_mouseup = move |_ev: web_sys::MouseEvent| {
        // Commit autofill if active, then reset drag state unconditionally.
        if let DragState::Extending { to_row, to_col } = state.drag.get_untracked() {
            model.update_value(|m| {
                m.pause_evaluation();
                let view = m.get_selected_view();
                let sheet = view.sheet;
                let [r1, c1, r2, c2] = view.range;
                let (r_min, r_max) = (r1.min(r2), r1.max(r2));
                let (c_min, c_max) = (c1.min(c2), c1.max(c2));
                let area = Area {
                    sheet,
                    row: r_min,
                    column: c_min,
                    height: r_max - r_min + 1,
                    width: c_max - c_min + 1,
                };
                // Fill rows if the drag crossed a row boundary, else fill columns.
                if to_row < r_min || to_row > r_max {
                    warn_if_err(m.auto_fill_rows(&area, to_row), "auto_fill_rows");
                } else {
                    warn_if_err(m.auto_fill_columns(&area, to_col), "auto_fill_columns");
                }
                m.resume_evaluation();
                m.evaluate();
            });
            // Autofill wrote content - emit a content event so canvas and
            // formula bar both repaint with the filled values.
            let sheet = model.with_value(|m| m.get_selected_view().sheet);
            let (start_row, start_col, end_row, end_col) = model.with_value(|m| {
                let [r1, c1, r2, c2] = m.get_selected_view().range;
                (r1, c1, r2, c2)
            });
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        state.drag.set(DragState::Idle);
    };

    // dblclick: enter edit mode with existing cell content
    let on_dblclick = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;
        if x < HEADER_COL_WIDTH || y < HEADER_ROW_HEIGHT {
            return;
        }
        // mousedown already navigated the active cell to the clicked position.
        model.with_value(|m| {
            let ac = m.active_cell();
            let text = m.active_cell_content();
            state.editing_cell.set(Some(EditingCell {
                address: CellAddress {
                    sheet: ac.sheet,
                    row: ac.row,
                    column: ac.column,
                },
                text,
                mode: EditMode::Edit,
                focus: EditFocus::Cell,
            }));
        });
    };

    // contextmenu: right-click on column/row header
    // TODO: create context menu component
    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        let x = ev.offset_x() as f64;
        let y = ev.offset_y() as f64;

        let target = if y < HEADER_ROW_HEIGHT && x >= HEADER_COL_WIDTH {
            // Column header
            Some(HeaderContextMenu::Column(model.with_value(|m| {
                let v = m.get_selected_view();
                let fg = frozen_geometry(m, v.sheet);
                pixel_to_col(m, v.sheet, v.left_column, x, &fg)
            })))
        } else if x < HEADER_COL_WIDTH && y >= HEADER_ROW_HEIGHT {
            // Row header
            Some(HeaderContextMenu::Row(model.with_value(|m| {
                let v = m.get_selected_view();
                let fg = frozen_geometry(m, v.sheet);
                pixel_to_row(m, v.sheet, v.top_row, y, &fg)
            })))
        } else {
            None
        };

        if let Some(target) = target {
            ev.prevent_default();
            state.context_menu.set(Some(ContextMenuState {
                x: ev.client_x(),
                y: ev.client_y(),
                target,
            }));
        }
    };

    // wheel: scroll with delta-magnitude awareness
    // Trackpads emit many small-delta events; physical wheels emit large ones.
    // Use arrow-style scroll for small deltas so trackpad users aren't thrown a
    // full page per gesture. Also handle delta_x for horizontal trackpad swipes.
    let on_wheel = move |ev: web_sys::WheelEvent| {
        ev.prevent_default();
        let dy = ev.delta_y();
        let dx = ev.delta_x();
        model.update_value(|m| {
            if dx.abs() > dy.abs() {
                // Predominantly horizontal - trackpad swipe.
                if dx > 0.0 {
                    m.nav_arrow(ArrowKey::Right);
                } else {
                    m.nav_arrow(ArrowKey::Left);
                }
            } else if dy.abs() < 100.0 {
                // Small vertical delta -> single-row scroll (trackpad).
                if dy > 0.0 {
                    m.nav_arrow(ArrowKey::Down);
                } else {
                    m.nav_arrow(ArrowKey::Up);
                }
            } else {
                // Large vertical delta -> page scroll (mouse wheel).
                if dy > 0.0 {
                    m.nav_page(PageDir::Down);
                } else {
                    m.nav_page(PageDir::Up);
                }
            }
        });
        let (sheet, top_row, left_col) = model.with_value(|m| {
            let v = m.get_selected_view();
            (v.sheet, v.top_row, v.left_column)
        });
        state.emit_event(SpreadsheetEvent::Navigation(
            NavigationEvent::ViewportScrolled {
                sheet,
                top_row,
                left_col,
            },
        ));
    };

    view! {
        <div node_ref=container_ref class="worksheet-container">
            <canvas
                node_ref=canvas_ref
                role="application"
                aria-label="Spreadsheet grid"
                class=move || {
                    match state.drag.get() {
                        DragState::ResizingCol { .. } => "worksheet-canvas resize-col",
                        DragState::ResizingRow { .. } => "worksheet-canvas resize-row",
                        DragState::Idle
                        | DragState::Selecting
                        | DragState::Extending { .. }
                        | DragState::Pointing { .. } => "worksheet-canvas",
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

// on_mousedown helpers
/// Start a column resize if the click lands within `HIT_ZONE` of a column
/// boundary in the header row. Returns `true` if a resize was started.
fn try_begin_col_resize(
    ev: &web_sys::MouseEvent,
    x: f64,
    model: ModelStore,
    state: WorkbookState,
) -> bool {
    if let Some(col) =
        model.with_value(|m| crate::canvas::geometry::find_col_boundary_at(m, x, HIT_ZONE))
    {
        state.drag.set(DragState::ResizingCol { col, x });
        ev.prevent_default();
        true
    } else {
        false
    }
}

/// Start a row resize if the click lands within `HIT_ZONE` of a row
/// boundary in the header column. Returns `true` if a resize was started.
fn try_begin_row_resize(
    ev: &web_sys::MouseEvent,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) -> bool {
    if let Some(row) =
        model.with_value(|m| crate::canvas::geometry::find_row_boundary_at(m, y, HIT_ZONE))
    {
        state.drag.set(DragState::ResizingRow { row, y });
        ev.prevent_default();
        true
    } else {
        false
    }
}

/// Click on the top-left corner cell: select the entire sheet.
fn handle_corner_click(model: ModelStore, state: WorkbookState) {
    model.update_value(|m| {
        m.nav_select_all();
    });
    state.editing_cell.set(None);
    let (sheet, start_row, start_col, end_row, end_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        let [r1, c1, r2, c2] = v.range;
        (v.sheet, r1, c1, r2, c2)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        },
    ));
}

/// Click on a column header: select the entire column, or extend the current
/// selection if Shift is held.
fn handle_col_header_click(
    ev: &web_sys::MouseEvent,
    x: f64,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        let view = m.get_selected_view();
        let sheet = view.sheet;
        let fg = frozen_geometry(m, sheet);
        let col = pixel_to_col(m, sheet, view.left_column, x, &fg);
        if ev.shift_key() {
            m.nav_extend_selection(1, col);
        } else {
            m.nav_select_column(col);
        }
    });
    state.editing_cell.set(None);
    let (sheet, start_row, start_col, end_row, end_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        let [r1, c1, r2, c2] = v.range;
        (v.sheet, r1, c1, r2, c2)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        },
    ));
}

/// Click on a row header: select the entire row, or extend the current
/// selection if Shift is held.
fn handle_row_header_click(
    ev: &web_sys::MouseEvent,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        let view = m.get_selected_view();
        let sheet = view.sheet;
        let fg = frozen_geometry(m, sheet);
        let row = pixel_to_row(m, sheet, view.top_row, y, &fg);
        if ev.shift_key() {
            m.nav_extend_selection(row, 1);
        } else {
            m.nav_select_row(row);
        }
    });
    state.editing_cell.set(None);
    let (sheet, start_row, start_col, end_row, end_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        let [r1, c1, r2, c2] = v.range;
        (v.sheet, r1, c1, r2, c2)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        },
    ));
}

/// Click in the cell area: handles point-mode formula entry, autofill handle
/// drag start, Shift-click range extension, and regular single-cell navigation.
fn handle_cell_click(
    ev: &web_sys::MouseEvent,
    x: f64,
    y: f64,
    model: ModelStore,
    state: WorkbookState,
) {
    // Read model state (row, col, autofill hit-test) without holding a
    // mutable borrow, so signal writes below don't interleave with the lock.
    let (row, col, near_handle) = model.with_value(|m| {
        let view = m.get_selected_view();
        let sheet = view.sheet;
        let fg = frozen_geometry(m, sheet);
        let col = pixel_to_col(m, sheet, view.left_column, x, &fg);
        let row = pixel_to_row(m, sheet, view.top_row, y, &fg);
        let handle = autofill_handle_pos(m);
        let near_handle = (x - handle.x).abs() <= AUTOFILL_HANDLE_PX
            && (y - handle.y).abs() <= AUTOFILL_HANDLE_PX;
        (row, col, near_handle)
    });

    // Point mode: intercept click during formula entry.
    // When the cursor is at a syntactically valid reference position inside
    // a formula, clicking a cell inserts/replaces the reference rather than
    // committing the edit and navigating away.
    if let Some(ref edit) = state.editing_cell.get_untracked() {
        if edit.mode == EditMode::Accept {
            let cursor = get_formula_cursor();
            let current_drag = state.drag.get_untracked();
            let already_pointing = matches!(current_drag, DragState::Pointing { .. });
            if already_pointing || is_in_reference_mode(&edit.text, cursor) {
                let sheet = model.with_value(|m| m.active_cell().sheet);
                let ref_str = range_ref_str(row, col, row, col, sheet, sheet, "");
                let prev_span = if let DragState::Pointing { ref_span, .. } = state.drag.get() {
                    Some(ref_span)
                } else {
                    None
                };
                let text = edit.text.clone();
                let (new_text, new_start, new_end) = splice_ref(&text, cursor, &ref_str, prev_span);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.text = new_text;
                    }
                });
                state.drag.set(DragState::Pointing {
                    range: SheetRect::from_cell(row, col),
                    ref_span: (new_start, new_end),
                });
                state.request_redraw();
                return;
            }
        }
    }

    if near_handle {
        // Begin autofill drag - don't change the selection.
        state.drag.set(DragState::Extending {
            to_row: row,
            to_col: col,
        });
    } else if ev.shift_key() {
        // Shift-click extends the range from the current anchor.
        model.update_value(|m| {
            m.nav_extend_selection(row, col);
        });
        state.drag.set(DragState::Selecting);
    } else {
        model.update_value(|m| {
            m.nav_set_cell(row, col);
        });
        state.drag.set(DragState::Selecting);
    }

    state.editing_cell.set(None);

    // Emit the appropriate navigation event so toolbar/formula-bar
    // update and the canvas repaints via visual_events.
    if near_handle {
        // Autofill start: drag state change alone triggers the Effect.
    } else {
        let sheet = model.with_value(|m| m.get_selected_view().sheet);
        if ev.shift_key() {
            let (start_row, start_col, end_row, end_col) = model.with_value(|m| {
                let [r1, c1, r2, c2] = m.get_selected_view().range;
                (r1, c1, r2, c2)
            });
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged {
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                },
            ));
        } else {
            let address = model.with_value(|m| {
                let v = m.get_selected_view();
                CellAddress {
                    sheet,
                    row: v.row,
                    column: v.column,
                }
            });
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionChanged { address },
            ));
        }
    }
}
