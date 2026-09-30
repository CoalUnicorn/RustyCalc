//! `handle_mousedown`: hit-test -> drag start or click dispatch.
//!
//! Resize handles are probed first (they straddle the header/cell seam
//! by `HIT_ZONE` px), then the normal hit-test routes to the four click
//! helpers in `click.rs`, starts a formula-reference drag, or — for a
//! Ctrl/Cmd-click on a link cell — follows the hyperlink.

use crate::coord::CellArea;
use crate::input::link::activate_link;
use crate::state::{DragState, ModelStore, StatusMessage, WorkbookState};
use iron_canvas_core::chrome::hit::{HitTest, ResizeTarget};
use leptos::prelude::WithValue;

use super::header_span::{Axis, full_header_span};

use super::click::{
    handle_cell_click, handle_col_header_click, handle_corner_click, handle_row_header_click,
};
use super::cursor_hint::HIT_ZONE;
use super::formula_ref::handle_formula_ref_mousedown;
use super::{CanvasHandle, with_canvas};

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
        let area = model.with_value(|m| CellArea::from_view(m));
        match target {
            ResizeTarget::ColumnEdge(col) => {
                let span = full_header_span(area, col, Axis::Col);
                state.drag.set(DragState::ResizingCol { col, span, x });
            }
            ResizeTarget::RowEdge(row) => {
                let span = full_header_span(area, row, Axis::Row);
                state.drag.set(DragState::ResizingRow { row, span, y });
            }
        }
        ev.prevent_default();
        return;
    }

    // 2. Click target.
    let hit = with_canvas(icv, |ic| ic.hit_test(x, y)).unwrap_or(HitTest::Outside);
    match hit {
        HitTest::Corner => handle_corner_click(model, state),
        HitTest::ColumnHeader(col) => handle_col_header_click(&ev, col, model, state),
        HitTest::RowHeader(row) => handle_row_header_click(&ev, row, model, state),
        HitTest::AutofillHandle { row, column } => {
            handle_cell_click(&ev, row, column, true, model, state)
        }
        HitTest::Cell { row, column } => {
            // Ctrl/Cmd-click follows the link under the pointer instead of
            // selecting. An active drag, edit, or point-mode owns the click:
            // the link is only followed from a clean idle mousedown.
            let link_click = (ev.ctrl_key() || ev.meta_key())
                && state.drag.get_untracked() == DragState::Idle
                && state.editing_cell.get_untracked().is_none()
                && with_canvas(icv, |ic| ic.link_at(row, column))
                    .flatten()
                    .is_some();
            if link_click {
                if let Err(e) = activate_link(model, &state, icv, row, column) {
                    state.status.set(Some(StatusMessage::Error(e.to_string())));
                }
                ev.prevent_default();
                return;
            }
            handle_cell_click(&ev, row, column, false, model, state)
        }
        HitTest::FormulaRef {
            ref_idx,
            zone,
            grab_row,
            grab_column,
        } => {
            handle_formula_ref_mousedown(&ev, ref_idx, zone, grab_row, grab_column, model, state);
        }
        HitTest::Outside => {}
    }
}
