//! `handle_mousemove`: drag continuation + hover hint.
//!
//! Two top-level match blocks: the first applies resize deltas
//! (ResizingCol/ResizingRow); the second runs the autoscroll edge
//! check and dispatches the remaining drag modes (Selecting,
//! Extending, Pointing, DraggingFormulaRef). Idle mousemove only
//! updates `state.cursor_hint`.

use leptos::prelude::*;
use wasm_bindgen::{JsCast, closure::Closure};

use crate::coord::{CellAddress, CellArea, RefNode, SheetRange};
use crate::events::{FormatEvent, NavigationEvent, SpreadsheetEvent};
use crate::input::error::StructError;
use crate::input::formula::splice_ref;
use crate::model::{ArrowKey, EvaluationMode, FormulaAnalyzer, Navigator, SheetRoster, try_mutate};
use crate::state::{DragState, ModelStore, RefOverride, StatusMessage, WorkbookState};
use iron_canvas_core::{
    geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, LAST_COLUMN, LAST_ROW},
    types::ui::HitTest,
};
use iron_canvas_web::PixelRect;
use ironcalc_base::UserModel;

use super::cursor_hint::compute_cursor_hint;
use super::formula_ref::dragged_ref_range;
use super::{CanvasHandle, with_canvas};

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
    // nav_expand_selection (-> on_expand_selected_range / Shift+Arrow) extends
    // the range AND calls set_top_left_visible_cell when the endpoint goes off-screen.
    // nav_extend_selection (-> on_area_selecting) only updates the range with no scroll.
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
            state.scroll_into_view.set_value(true);
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::SelectionRangeChanged { sheet_area },
            ));
        }
        // Deliberately does *not* arm scroll_into_view: this arm scrolls the
        // viewport while leaving the selection put, so following the active
        // cell would drag the view straight back.
        DragState::Extending { .. } => {
            // set_top_left_visible_cell scrolls without touching the selection
            // range — the source cells the autofill will fill from must stay intact.
            let (mx, my) = state.autoscroll.pos.get_value();
            model.update_value(|m| {
                let view = m.get_selected_view();
                let new_top = (view.top_row + dy).clamp(1, LAST_ROW);
                let new_left = (view.left_column + dx).clamp(1, LAST_COLUMN);
                if let Err(e) = m.set_top_left_visible_cell(new_top, new_left) {
                    web_sys::console::warn_1(&format!("[rustycalc nav] scroll: {e}").into());
                }
            });
            // Resolve the new drag-target against the *previous* painted frame.
            // The scroll mutation above won't be reflected on canvas until the
            // next renderPending — so hit_test against last_frame matches
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

/// Compute the scroll direction from the cursor position and the *scrollable
/// pane's* bounds, then start or stop the auto-scroll timer accordingly.
///
/// `pane` is the region past the frozen bands — never the whole canvas. With
/// panes frozen, the pane's near edges sit a frozen band's worth in from the
/// canvas origin, and a zone measured off the canvas would fire deep inside
/// the frozen band, which doesn't scroll.
///
/// If the cursor has moved back into the safe zone, the running timer is cancelled.
/// If a timer is already running and the direction changed, the new direction is
/// stored and the existing timer picks it up on the next tick — no restart needed.
fn update_autoscroll(
    x: f64,
    y: f64,
    pane: Option<PixelRect>,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let Some(pane) = pane else {
        return;
    };
    // An axis whose frozen band fills the canvas has no scrollable extent, so
    // its near and far edge zones would overlap and latch a direction.
    let dx = if pane.width <= 0 {
        0
    } else if x > f64::from(pane.right()) - AUTOSCROLL_ZONE {
        1
    } else if x < f64::from(pane.top_left.x) + AUTOSCROLL_ZONE {
        -1
    } else {
        0
    };
    let dy = if pane.height <= 0 {
        0
    } else if y > f64::from(pane.bottom()) - AUTOSCROLL_ZONE {
        1
    } else if y < f64::from(pane.top_left.y) + AUTOSCROLL_ZONE {
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

/// the canvas). Reset drag state so the next interaction starts clean.
pub fn handle_mousemove(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;
    if ev.buttons() == 0 {
        state.autoscroll.cancel();
        state.drag.set(DragState::Idle);
        state.dragged_ref_override.set(None);
        let hint = compute_cursor_hint(icv, x, y);
        if state.hover_cursor.get_untracked() != hint {
            state.hover_cursor.set(hint);
        }
        return;
    }
    let sheet = model.with_value(UserModel::get_selected_sheet);

    match state.drag.get_untracked() {
        DragState::ResizingCol {
            col,
            span,
            x: last_x,
        } => {
            let delta = x - last_x;
            let result = try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    let current_w = m.get_column_width(sheet, col).unwrap_or(DEFAULT_COL_WIDTH);
                    let new_w = (current_w + delta).max(5.0);
                    m.set_columns_width(sheet, span.0, span.1, new_w)
                        .map_err(StructError::Engine)
                },
            );
            state.drag.set(DragState::ResizingCol { col, span, x });
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
        DragState::ResizingRow {
            row,
            span,
            y: last_y,
        } => {
            let delta = y - last_y;
            let result = try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    let current_h = m.get_row_height(sheet, row).unwrap_or(DEFAULT_ROW_HEIGHT);
                    let new_h = (current_h + delta).max(3.0);
                    m.set_rows_height(sheet, span.0, span.1, new_h)
                        .map_err(StructError::Engine)
                },
            );
            state.drag.set(DragState::ResizingRow { row, span, y });
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
        | DragState::Pointing { .. }
        | DragState::DraggingFormulaRef { .. } => {}
    }

    // Formula-ref drag must resolve the pointer to a cell BEFORE the
    // layer-aware `ic.hit_test` below — otherwise `FormulaRefsLayer`
    // claims the hit whenever the cursor re-enters its own painted rect
    // (i.e. shrink direction) and the let-else bails out, freezing the
    // drag. `pixel_to_cell` walks only the chrome's pane_set, so any
    // overlay above it is invisible to the resolution.
    if let DragState::DraggingFormulaRef {
        ref_idx,
        zone,
        anchor,
        grab_cell,
    } = state.drag.get_untracked()
    {
        let Some((row, col)) = with_canvas(icv, |ic| ic.pixel_to_cell(x, y)).flatten() else {
            return;
        };
        let cursor = CellAddress {
            sheet: anchor.sheet,
            row,
            column: col,
        };
        let new_range = dragged_ref_range(anchor, zone, grab_cell, cursor);
        state.dragged_ref_override.set(Some(RefOverride {
            idx: ref_idx,
            range: new_range,
        }));
        ev.prevent_default();
        return;
    }

    // Hit-test against the painted frame. Anything that isn't a Cell (header,
    // corner, autofill handle, off-canvas) means the drag-target sits outside
    // the scrollable grid — bail and let the autoscroll timer (if any)
    // continue to advance the viewport on its own cadence.
    let Some(HitTest::Cell { row, column: col }) = with_canvas(icv, |ic| ic.hit_test(x, y)) else {
        return;
    };

    // `None` until the first paint (no frame, so no pane geometry) — the drag
    // state below still updates, only the edge-scroll is skipped.
    let pane = with_canvas(icv, |ic| ic.scroll_pane_rect()).flatten();

    match state.drag.get_untracked() {
        DragState::Extending { .. } => {
            update_autoscroll(x, y, pane, model, state, icv);
            state.drag.set(DragState::Extending {
                to_row: row,
                to_col: col,
            });
        }
        DragState::Pointing {
            ref_node: pr,
            ref_text: ref_span,
        } => {
            // Pinned edit origin: `pr`'s relative offsets were stored against
            // it, and the stringify ctx must match the offset base. The
            // dragged area itself lives on the visible sheet (`sheet`), which
            // qualifies the ref when it differs from the origin.
            let editing = state
                .editing_cell
                .get_untracked()
                .map(|e| e.address)
                .unwrap_or_else(|| model.with_value(CellAddress::from_view));
            let sheet_name = if sheet == editing.sheet {
                String::new()
            } else {
                model.with_value(|m| m.get_sheet_name(sheet as usize))
            };
            // Anchor corner is (r1, c1) of the currently-pointed range; mouse
            // position supplies the new trailing corner. Normalize so the
            // endpoints stay ordered after drag inversions (e.g. mouse crosses
            // the anchor).
            let anchor = pr.area(&editing).area;
            let dragged = CellArea {
                r1: anchor.r1,
                c1: anchor.c1,
                r2: row,
                c2: col,
            }
            .normalized()
            .with_sheet(sheet);
            let ref_node = RefNode::from_cell_area(dragged, editing, &sheet_name);
            let ref_str = ref_node.to_localized(&editing.as_stringify_ctx());

            if let Some(edit) = state.editing_cell.get_untracked() {
                let (new_text, ref_span) = splice_ref(&edit.text, ref_span, &ref_str);
                state.editing_cell.update(|c| {
                    if let Some(e) = c {
                        e.cursor = ref_span.end;
                        // Pinned origin anchor — drag may extend from a
                        // switched-to sheet.
                        e.formula_analysis =
                            model.with_value(|m| m.analyze_at(&new_text, e.address));
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
            update_autoscroll(x, y, pane, model, state, icv);
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
        // DraggingFormulaRef is handled by the early short-circuit above
        // (before the layer-aware ic.hit_test that would otherwise let
        // FormulaRefsLayer shadow the cell under the cursor).
        DragState::DraggingFormulaRef { .. }
        | DragState::Idle
        | DragState::ResizingCol { .. }
        | DragState::ResizingRow { .. } => {}
    }
}
