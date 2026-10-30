//! `handle_contextmenu`: right-click on header -> show context menu.
//!
//! Clicks in the cell grid are ignored — cell context menu not yet
//! implemented.

use leptos::prelude::*;

use crate::coord::CellArea;
use crate::state::{ContextMenuState, HeaderContextMenu, ModelStore, WorkbookState};
use iron_canvas_core::Point;
use iron_canvas_core::scene_geometry::GridHit;

use super::header_span::{Axis, full_header_span};
use super::{CanvasHandle, with_canvas};

/// Clicks in the cell grid are ignored — cell context menu not yet implemented.
pub fn handle_contextmenu(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let point = Point {
        x: ev.offset_x(),
        y: ev.offset_y(),
    };

    let target = match with_canvas(icv, model, state, |h| h.hit_test(point)).flatten() {
        Some(GridHit::ColumnHeader(col)) => Some(model.with_value(|m| {
            let area = CellArea::from_view(m);
            let (first, last) = full_header_span(area, col, Axis::Col);
            HeaderContextMenu::Column {
                col: first,
                count: last - first + 1,
            }
        })),
        Some(GridHit::RowHeader(row)) => Some(model.with_value(|m| {
            let area = CellArea::from_view(m);
            let (first, last) = full_header_span(area, row, Axis::Row);
            HeaderContextMenu::Row {
                row: first,
                count: last - first + 1,
            }
        })),
        _ => None,
    };

    let Some(target) = target else {
        return;
    };
    ev.prevent_default();
    state.context_menu.set(Some(ContextMenuState {
        x: ev.client_x(),
        y: ev.client_y(),
        target,
    }));
}
