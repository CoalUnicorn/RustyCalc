//! `handle_mousedown`: hit-test -> drag start or click dispatch.
//!
//! Resize handles are probed first (they straddle the header/cell seam
//! by `HIT_ZONE` px), then the normal hit-test routes to the four click
//! helpers in `click.rs` or — for a Ctrl/Cmd-click on a link cell — follows
//! the hyperlink.

use crate::coord::CellArea;
use crate::input::link::activate_link;
use crate::state::{DragState, ModelStore, StatusMessage, WorkbookState};
use iron_canvas_core::Point;
use iron_canvas_core::scene_geometry::{GridHit, GridResize};
use leptos::prelude::WithValue;

use super::header_span::{Axis, full_header_span};

use super::click::{
    handle_cell_click, handle_col_header_click, handle_corner_click, handle_row_header_click,
};
use super::cursor_hint::{HIT_ZONE, autofill_hit};
use super::formula_ref::{draggable_ref_indices, handle_formula_ref_mousedown};
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

    let point = Point {
        x: ev.offset_x(),
        y: ev.offset_y(),
    };

    // 1. Resize handle (column or row boundary in its header strip).
    if let Some(target) = with_canvas(icv, |h| h.resize_target(point, HIT_ZONE)).flatten() {
        let area = model.with_value(|m| CellArea::from_view(m));
        match target {
            GridResize::Column(col) => {
                let span = full_header_span(area, col, Axis::Col);
                state.drag.set(DragState::ResizingCol {
                    col,
                    span,
                    x: ev.offset_x() as f64,
                });
            }
            GridResize::Row(row) => {
                let span = full_header_span(area, row, Axis::Row);
                state.drag.set(DragState::ResizingRow {
                    row,
                    span,
                    y: ev.offset_y() as f64,
                });
            }
        }
        ev.prevent_default();
        return;
    }

    // Formula-reference overlays sit above cells. Start their drag before the
    // normal grid hit so the reference keeps its own pointer operation.
    let draggable_refs = state
        .editing_cell
        .get_untracked()
        .map(|edit| draggable_ref_indices(edit.formula_analysis.refs()))
        .unwrap_or_default();
    if let Some(hit) =
        with_canvas(icv, |h| h.formula_ref_hit_test(point, &draggable_refs)).flatten()
    {
        handle_formula_ref_mousedown(&ev, hit, state);
        return;
    }

    // 2. Click target.
    let hit = with_canvas(icv, |h| h.hit_test(point)).flatten();
    match hit {
        Some(GridHit::Corner) => handle_corner_click(model, state),
        Some(GridHit::ColumnHeader(col)) => handle_col_header_click(&ev, col, model, state),
        Some(GridHit::RowHeader(row)) => handle_row_header_click(&ev, row, model, state),
        Some(GridHit::Cell(coord)) => {
            // The autofill handle is not a hit-test class in the scene
            // geometry: the host derives it from the selection's
            // bottom-right corner square.
            let near_handle = with_canvas(icv, |h| autofill_hit(h, model, point)).unwrap_or(false);
            if near_handle {
                handle_cell_click(&ev, coord.row, coord.col, true, model, state);
                ev.prevent_default();
                return;
            }
            // Ctrl/Cmd-click follows the link under the pointer instead of
            // selecting. An active drag or edit owns the click: the link is
            // only followed from a clean idle mousedown.
            // The logical anchor, not the physical slot: over a merged range
            // the anchor owns the link and is the address the browser opens.
            let link_anchor = with_canvas(icv, |h| {
                h.display_cell_at(point)
                    .and_then(|cell| cell.link.map(|_| (cell.anchor.row, cell.anchor.col)))
            })
            .flatten();
            let link_click = (ev.ctrl_key() || ev.meta_key())
                && state.drag.get_untracked() == DragState::Idle
                && state.editing_cell.get_untracked().is_none()
                && link_anchor.is_some();
            if link_click {
                if let Err(e) = activate_link(model, &state, icv, point) {
                    state.status.set(Some(StatusMessage::Error(e.to_string())));
                }
                ev.prevent_default();
                return;
            }
            handle_cell_click(&ev, coord.row, coord.col, false, model, state)
        }
        None => {}
    }
}
