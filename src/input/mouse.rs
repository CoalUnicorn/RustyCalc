//! Mouse event handlers for the worksheet canvas.
//!
//! Most public functions follow the pattern used throughout `src/input/`:
//! pure logic that takes `(model: ModelStore, state: WorkbookState)` and
//! returns `()`. The two resize-begin helpers return `bool` to signal
//! whether a resize was started. The worksheet component holds thin
//! closures that delegate here.

use leptos::prelude::*;
use wasm_bindgen::{closure::Closure, JsCast};

use crate::coord::{Cell, CellRange, RefNode, SheetRange, TextRef};
use crate::events::{ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::StructError;
use crate::input::formula_analysis::is_in_reference_mode;
use crate::input::formula_input::splice_ref;
use crate::model::{try_mutate, ArrowKey, EvaluationMode, FrontendModel, PageDir};
use crate::state::{
    ContextMenuState, DragState, EditFocus, EditMode, EditingCell, HeaderContextMenu, ModelStore,
    StatusMessage, WorkbookState,
};
use iron_canvas::geometry::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, LAST_COLUMN, LAST_ROW};
use iron_canvas::{
    CanvasSize, HitTest, IronCanvas, ResizeTarget, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use ironcalc_base::UserModel;

/// Storage type for the IronCanvas orchestrator handle. `LocalStorage`
/// because `IronCanvas` is `!Send` (holds web_sys handles); `StoredValue`
/// because we don't want event listeners to subscribe to changes — the
/// handle is created once on mount, dropped on unmount.
pub type CanvasHandle = StoredValue<Option<IronCanvas>, LocalStorage>;

/// Read a value from the canvas handle. Used by every event handler that
/// needs to query the painted state — hit-tests, resize probes, cell
/// rectangles. Returns the closure's result wrapped in `Option`, since the
/// handle is `None` until both `<canvas>` elements mount and the lazy
/// rAF construction runs (see `worksheet.rs`).
fn with_canvas<R>(handle: CanvasHandle, f: impl FnOnce(&IronCanvas) -> R) -> Option<R> {
    handle.with_value(|slot| slot.as_ref().map(f))
}

/// Pixel tolerance for column/row resize hit-test in the header area.
const HIT_ZONE: f64 = 4.0;
/// Pixels from a canvas edge that activate auto-scroll during drag.
const AUTOSCROLL_ZONE: f64 = 40.0;
/// Milliseconds between auto-scroll ticks while the cursor is in the edge zone.
const AUTOSCROLL_MS: i32 = 80;

/// Called on each timer tick while a drag is active near a canvas edge.
///
/// Advances the viewport one step in the stored direction, re-resolves the
/// last-known mouse position into the shifted viewport, then updates the
/// active drag state so the canvas repaints without the user needing to move
/// the mouse.
fn autoscroll_tick(model: ModelStore, state: WorkbookState, icv: CanvasHandle) {
    let (dx, dy) = state.autoscroll.dir.get_value();
    if dx == 0 && dy == 0 {
        return;
    }
    let sheet = model.with_value(UserModel::get_selected_sheet);
    // nav_expand_selection (→ on_expand_selected_range / Shift+Arrow) extends
    // the range AND calls set_top_left_visible_cell when the endpoint goes off-screen.
    // nav_extend_selection (→ on_area_selecting) only updates the range with no scroll.
    // nav_arrow collapses the selection anchor — never use it during a drag.
    match state.drag.get_untracked() {
        DragState::Selecting => {
            model.update_value(|m| {
                if dx > 0 {
                    m.nav_expand_selection(ArrowKey::Right);
                }
                if dx < 0 {
                    m.nav_expand_selection(ArrowKey::Left);
                }
                if dy > 0 {
                    m.nav_expand_selection(ArrowKey::Down);
                }
                if dy < 0 {
                    m.nav_expand_selection(ArrowKey::Up);
                }
            });
            let sheet_area = model.with_value(SheetRange::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        }
        DragState::Extending { .. } => {
            // set_top_left_visible_cell scrolls without touching the selection
            // range — the source cells the autofill will fill from must stay intact.
            let (mx, my) = state.autoscroll.pos.get_value();
            model.update_value(|m| {
                let view = m.get_selected_view();
                let new_top = (view.top_row + dy).clamp(1, LAST_ROW);
                let new_left = (view.left_column + dx).clamp(1, LAST_COLUMN);
                let _ = m.set_top_left_visible_cell(new_top, new_left);
            });
            // Resolve the new drag-target against the *previous* painted frame.
            // The scroll mutation above won't be reflected on canvas until the
            // next paint_if_dirty — so hit_test against last_frame matches
            // what the user still sees.
            if let Some(HitTest::Cell { row, column }) = with_canvas(icv, |ic| ic.hit_test(mx, my))
            {
                state.drag.set(DragState::Extending {
                    to_row: row,
                    to_col: column,
                });
            }
        }
        _ => {
            state.autoscroll.cancel();
            return;
        }
    }
    let (top_row, left_col) = model.with_value(|m| {
        let v = m.get_selected_view();
        (v.top_row, v.left_column)
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::ViewportScrolled {
            sheet,
            top_row,
            left_col,
        },
    ));
}

/// Compute the scroll direction from the cursor position and the canvas bounds,
/// then start or stop the auto-scroll timer accordingly.
///
/// If the cursor has moved back into the safe zone, the running timer is cancelled.
/// If a timer is already running and the direction changed, the new direction is
/// stored and the existing timer picks it up on the next tick — no restart needed.
fn update_autoscroll(
    x: f64,
    y: f64,
    canvas_w: f64,
    canvas_h: f64,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let dx = if x > canvas_w - AUTOSCROLL_ZONE {
        1
    } else if x < HEADER_COL_WIDTH + AUTOSCROLL_ZONE {
        -1
    } else {
        0
    };
    let dy = if y > canvas_h - AUTOSCROLL_ZONE {
        1
    } else if y < HEADER_ROW_HEIGHT + AUTOSCROLL_ZONE {
        -1
    } else {
        0
    };
    state.autoscroll.pos.set_value((x, y));
    state.autoscroll.dir.set_value((dx, dy));
    if dx == 0 && dy == 0 {
        state.autoscroll.cancel();
        return;
    }
    if state.autoscroll.id.get_value().is_some() {
        return; // timer already running; direction update above is enough
    }
    // `cb.forget()` hands ownership to the JS GC for the lifetime of the interval.
    let cb = Closure::<dyn FnMut()>::new(move || autoscroll_tick(model, state, icv));
    let id = leptos::prelude::window()
        .set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref::<web_sys::js_sys::Function>(),
            AUTOSCROLL_MS,
        )
        .unwrap_or(-1);
    cb.forget();
    state.autoscroll.id.set_value(Some(id));
}

/// Click on the top-left corner cell: select the entire sheet.
pub fn handle_corner_click(model: ModelStore, state: WorkbookState) {
    web_sys::console::time_with_label("corner:nav_select_all");
    model.update_value(|m| {
        m.nav_select_all();
    });
    web_sys::console::time_end_with_label("corner:nav_select_all");

    web_sys::console::time_with_label("corner:editing_cell");

    state.editing_cell.set(None);
    web_sys::console::time_end_with_label("corner:editing_cell");

    web_sys::console::time_with_label("corner:from_view");
    let sheet_area = model.with_value(SheetRange::from_view);
    web_sys::console::time_end_with_label("corner:from_view");

    web_sys::console::time_with_label("corner:emit_event");
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
    web_sys::console::time_end_with_label("corner:emit_event");
}

/// Click on a column header: select the entire column, or extend the current
/// selection if Shift is held. `col` is the column index resolved by the
/// dispatcher's `IronCanvas::hit_test` against the painted frame.
pub fn handle_col_header_click(
    ev: &web_sys::MouseEvent,
    col: i32,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        if ev.shift_key() {
            m.nav_extend_column_selection(col);
        } else {
            m.nav_select_column(col);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetRange::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click on a row header: select the entire row, or extend the current
/// selection if Shift is held.
pub fn handle_row_header_click(
    ev: &web_sys::MouseEvent,
    row: i32,
    model: ModelStore,
    state: WorkbookState,
) {
    model.update_value(|m| {
        if ev.shift_key() {
            m.nav_extend_row_selection(row);
        } else {
            m.nav_select_row(row);
        }
    });
    state.editing_cell.set(None);
    let sheet_area = model.with_value(SheetRange::from_view);
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged { sheet_area },
    ));
}

/// Click in the cell area: handles point-mode formula entry, autofill handle
/// drag start, Shift-click range extension, and regular single-cell navigation.
///
/// `row` / `col` are the cell under the cursor, resolved upstream by
/// `IronCanvas::hit_test` against the painted frame. `near_handle` is `true`
/// iff the dispatcher classified the hit as `HitTest::AutofillHandle`.
pub fn handle_cell_click(
    ev: &web_sys::MouseEvent,
    row: i32,
    col: i32,
    near_handle: bool,
    model: ModelStore,
    state: WorkbookState,
) {
    // Point mode: intercept click during formula entry.
    // When the cursor is at a syntactically valid reference position inside
    // a formula, clicking a cell inserts/replaces the reference rather than
    // committing the edit and navigating away.
    if let Some(ref edit) = state.editing_cell.get_untracked() {
        let already_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });
        let may_point = edit.mode == EditMode::Accept || edit.text_dirty || already_pointing;
        if may_point {
            let cursor = edit.cursor;
            // Caret-hit: if the cursor sits on an existing resolved ref,
            // the click REPLACES that ref in place — preserving its `$`
            // flags and sheet qualification via `relocate_to`.
            let caret_hit = if !already_pointing {
                edit.formula_analysis.refs_at_cursor(cursor).next().cloned()
            } else {
                None
            };
            if already_pointing || caret_hit.is_some() || is_in_reference_mode(&edit.text, cursor) {
                let editing = model.with_value(Cell::from_view);
                let (ref_node, prev_span) = if let Some(hit) = caret_hit {
                    (hit.ref_node.relocate_to(row, col, &editing), Some(hit.span))
                } else if let DragState::Pointing { ref_text, .. } = state.drag.get_untracked() {
                    (
                        RefNode::from_cell_area(
                            SheetRange::from_cell(editing.sheet, row, col),
                            editing,
                            "",
                        ),
                        Some(ref_text),
                    )
                } else {
                    (
                        RefNode::from_cell_area(
                            SheetRange::from_cell(editing.sheet, row, col),
                            editing,
                            "",
                        ),
                        None,
                    )
                };
                let ref_str = ref_node.to_localized(&editing.as_stringify_ctx());
                let text = edit.text.clone();
                let (new_text, ref_text) =
                    splice_ref(&text, prev_span.unwrap_or(TextRef::at(cursor)), &ref_str);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.cursor = ref_text.end;
                        e.text = new_text.clone();
                        e.formula_analysis = model.with_value(|m| m.analyze_in_context(&new_text));
                    }
                });
                state.drag.set(DragState::Pointing { ref_node, ref_text });
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
    // Autofill start: drag state change alone triggers the canvas repaint; no navigation event.
    if !near_handle {
        if ev.shift_key() {
            let sheet_area = model.with_value(SheetRange::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        } else {
            let address = model.with_value(Cell::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionChanged { address },
            ));
        }
    }
}

/// Dispatch a mousedown event to the appropriate region handler.
///
/// Resize-handle proximity wins over plain header clicks (the cursor is on
/// the boundary, not the strip body), so it is probed first. Everything
/// else falls out of `IronCanvas::hit_test` against the painted frame —
/// the renderer owns the layout, so it owns the dispatch.
pub fn handle_mousedown(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    // Only handle left-click (button 0); right-click is handled by handle_contextmenu.
    if ev.button() != 0 {
        return;
    }

    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

    // 1. Resize handle (column or row boundary in its header strip).
    if let Some(target) = with_canvas(icv, |ic| ic.resize_handle_at(x, y, HIT_ZONE)).flatten() {
        match target {
            ResizeTarget::Column(col) => state.drag.set(DragState::ResizingCol { col, x }),
            ResizeTarget::Row(row) => state.drag.set(DragState::ResizingRow { row, y }),
        }
        ev.prevent_default();
        return;
    }

    // 2. Click target.
    let hit = with_canvas(icv, |ic| ic.hit_test(x, y)).unwrap_or(HitTest::Outside);
    match hit {
        HitTest::Corner => handle_corner_click(model, state),
        HitTest::ColHeader(col) => handle_col_header_click(&ev, col, model, state),
        HitTest::RowHeader(row) => handle_row_header_click(&ev, row, model, state),
        HitTest::AutofillHandle { row, column } => {
            handle_cell_click(&ev, row, column, true, model, state)
        }
        HitTest::Cell { row, column } => handle_cell_click(&ev, row, column, false, model, state),
        HitTest::Outside => {}
    }
}

/// Expand selection, update resize drag, or update autofill/point-mode
/// preview while a button is held.
///
/// If no button is held when this fires, mouseup was missed (pointer left
/// the canvas). Reset drag state so the next interaction starts clean.
pub fn handle_mousemove(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    if ev.buttons() == 0 {
        state.autoscroll.cancel();
        state.drag.set(DragState::Idle);
        return;
    }
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;
    let sheet = model.with_value(UserModel::get_selected_sheet);

    match state.drag.get_untracked() {
        DragState::ResizingCol { col, x: last_x } => {
            let delta = x - last_x;
            let result = try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    let current_w = m.get_column_width(sheet, col).unwrap_or(DEFAULT_COL_WIDTH);
                    let new_w = (current_w + delta).max(5.0);
                    m.set_columns_width(sheet, col, col, new_w)
                        .map_err(StructError::Engine)
                },
            );
            state.drag.set(DragState::ResizingCol { col, x });
            if let Err(e) = result {
                state.status.set(Some(StatusMessage::Error(e.to_string())));
                ev.prevent_default();
                return;
            }
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
            let result = try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    let current_h = m.get_row_height(sheet, row).unwrap_or(DEFAULT_ROW_HEIGHT);
                    let new_h = (current_h + delta).max(3.0);
                    m.set_rows_height(sheet, row, row, new_h)
                        .map_err(StructError::Engine)
                },
            );
            state.drag.set(DragState::ResizingRow { row, y });
            if let Err(e) = result {
                state.status.set(Some(StatusMessage::Error(e.to_string())));
                ev.prevent_default();
                return;
            }
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

    // Hit-test against the painted frame. Anything that isn't a Cell (header,
    // corner, autofill handle, off-canvas) means the drag-target sits outside
    // the scrollable grid — bail and let the autoscroll timer (if any)
    // continue to advance the viewport on its own cadence.
    let Some(HitTest::Cell { row, column: col }) = with_canvas(icv, |ic| ic.hit_test(x, y)) else {
        return;
    };

    // Sentinel size when the canvas isn't mounted yet: f64::MAX guarantees the
    // cursor is never within AUTOSCROLL_ZONE of any edge, so update_autoscroll
    // is a no-op pre-mount instead of latching onto a zero-sized canvas.
    let CanvasSize {
        w: canvas_w,
        h: canvas_h,
    } = with_canvas(icv, |ic| ic.canvas_size()).unwrap_or(CanvasSize {
        w: f64::MAX,
        h: f64::MAX,
    });

    match state.drag.get_untracked() {
        DragState::Extending { .. } => {
            update_autoscroll(x, y, canvas_w, canvas_h, model, state, icv);
            state.drag.set(DragState::Extending {
                to_row: row,
                to_col: col,
            });
        }
        DragState::Pointing {
            ref_node: pr,
            ref_text: ref_span,
        } => {
            let editing = model.with_value(Cell::from_view);
            // Anchor corner is (r1, c1) of the currently-pointed range; mouse
            // position supplies the new trailing corner. Normalize so the
            // endpoints stay ordered after drag inversions (e.g. mouse crosses
            // the anchor).
            let anchor = pr.area(&editing).area;
            let dragged = CellRange {
                r1: anchor.r1,
                c1: anchor.c1,
                r2: row,
                c2: col,
            }
            .normalized()
            .with_sheet(sheet);
            let ref_node = RefNode::from_cell_area(dragged, editing, "");
            let ref_str = ref_node.to_localized(&editing.as_stringify_ctx());

            if let Some(edit) = state.editing_cell.get_untracked() {
                let (new_text, ref_span) = splice_ref(&edit.text, ref_span, &ref_str);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.cursor = ref_span.end;
                        e.formula_analysis = model.with_value(|m| m.analyze_in_context(&new_text));
                        e.text = new_text;
                    }
                });

                state.drag.set(DragState::Pointing {
                    ref_node,
                    ref_text: ref_span,
                });
            }
        }
        DragState::Selecting => {
            update_autoscroll(x, y, canvas_w, canvas_h, model, state, icv);
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
            let sheet_area = model.with_value(SheetRange::from_view);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        }
        DragState::Idle | DragState::ResizingCol { .. } | DragState::ResizingRow { .. } => {}
    }
}

/// Commit an autofill drag on button release, then reset drag state.
///
/// If no autofill drag was active, this is a no-op beyond resetting to `Idle`.
pub fn handle_mouseup(_ev: web_sys::MouseEvent, model: ModelStore, state: WorkbookState) {
    state.autoscroll.cancel();
    let was_pointing = matches!(state.drag.get_untracked(), DragState::Pointing { .. });

    if let DragState::Extending { to_row, to_col } = state.drag.get_untracked() {
        match try_mutate(
            model,
            EvaluationMode::Immediate,
            |m| -> Result<(), StructError> {
                let norm = CellRange::from_view(m).normalized();
                let area = norm.to_area(m.get_selected_sheet());
                if to_row < norm.r1 || to_row > norm.r2 {
                    m.auto_fill_rows(&area, to_row)
                        .map_err(StructError::Engine)?;
                } else {
                    m.auto_fill_columns(&area, to_col)
                        .map_err(StructError::Engine)?;
                }
                Ok(())
            },
        ) {
            Ok(()) => {
                let sheet_area = model.with_value(SheetRange::from_view);
                state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                    sheet_area,
                }));
            }
            Err(e) => state.status.set(Some(StatusMessage::Error(e.to_string()))),
        }
    }
    state.drag.set(DragState::Idle);
    // After a point-mode drag, return focus to the formula input so the user
    // can continue typing the formula without clicking again.
    if was_pointing {
        state.refocus_formula_input();
    }
}

/// Right-click on a column or row header: store position and target for
/// the header context menu overlay.
///
/// Clicks in the cell grid are ignored — cell context menu not yet implemented.
pub fn handle_contextmenu(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

    let target = match with_canvas(icv, |ic| ic.hit_test(x, y)) {
        Some(HitTest::ColHeader(col)) => Some(model.with_value(|m| {
            let area = CellRange::from_view(m).normalized();
            // Multi-column selection if the clicked col is inside a full-column range.
            let (col, count) = if area.r2 >= LAST_ROW && area.c1 <= col && col <= area.c2 {
                (area.c1, area.c2 - area.c1 + 1)
            } else {
                (col, 1)
            };
            HeaderContextMenu::Column { col, count }
        })),
        Some(HitTest::RowHeader(row)) => Some(model.with_value(|m| {
            let area = CellRange::from_view(m).normalized();
            // Multi-row selection if the clicked row is inside a full-row range.
            let (row, count) = if area.c2 >= LAST_COLUMN && area.r1 <= row && row <= area.r2 {
                (area.r1, area.r2 - area.r1 + 1)
            } else {
                (row, 1)
            };
            HeaderContextMenu::Row { row, count }
        })),
        _ => None,
    };

    if let Some(target) = target {
        ev.prevent_default();
        state.context_menu.set(Some(ContextMenuState {
            x: ev.client_x(),
            y: ev.client_y(),
            target,
        }));
    }
}

/// Scroll the viewport on mouse wheel or trackpad swipe.
///
/// Trackpads emit many small-delta events; physical wheels emit large ones.
/// Small vertical deltas (< 100px) use single-row scroll; large ones use
/// page scroll. Horizontal deltas scroll left/right by one column.
pub fn handle_wheel(ev: web_sys::WheelEvent, model: ModelStore, state: WorkbookState) {
    ev.prevent_default();
    let dy = ev.delta_y();
    let dx = ev.delta_x();
    model.update_value(|m| {
        if dx.abs() > dy.abs() {
            // Predominantly horizontal — trackpad swipe.
            if dx > 0.0 {
                m.nav_arrow(ArrowKey::Right);
            } else {
                m.nav_arrow(ArrowKey::Left);
            }
        } else if dy.abs() < 100.0 {
            // Small vertical delta — single-row scroll (trackpad).
            if dy > 0.0 {
                m.nav_arrow(ArrowKey::Down);
            } else {
                m.nav_arrow(ArrowKey::Up);
            }
        } else {
            // Large vertical delta — page scroll (mouse wheel).
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
}

/// Enter edit mode with the existing cell content on double-click.
///
/// The preceding mousedown already navigated to the target cell, so this
/// only needs to open the editor at the current address — but only for
/// double-clicks that land inside the cell grid (or the autofill handle,
/// which sits at a cell corner). Header / corner double-clicks fall through.
pub fn handle_dblclick(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;
    match with_canvas(icv, |ic| ic.hit_test(x, y)) {
        Some(HitTest::Cell { .. }) | Some(HitTest::AutofillHandle { .. }) => {}
        _ => return,
    }
    model.with_value(|m| {
        let ac = m.active_cell();
        let text = m.active_cell_content();
        let formula_analysis = model.with_value(|m| m.analyze_in_context(&text));
        state.editing_cell.set(Some(EditingCell {
            address: Cell {
                sheet: ac.sheet,
                row: ac.row,
                column: ac.column,
            },
            cursor: text.len(),
            text,
            mode: EditMode::Edit,
            focus: EditFocus::Cell,
            text_dirty: false,
            formula_analysis,
        }));
    });
}
