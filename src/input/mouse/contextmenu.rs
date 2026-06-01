//! `handle_contextmenu`: right-click on header → show context menu.
//!
//! Clicks in the cell grid are ignored — cell context menu not yet
//! implemented.

use leptos::prelude::*;

use crate::coord::CellArea;
use crate::state::{ContextMenuState, HeaderContextMenu, ModelStore, WorkbookState};
use iron_canvas_core::types::ui::HitTest;

use super::header_span::{Axis, full_header_span};
use super::{CanvasHandle, with_canvas};

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
            let area = CellArea::from_view(m);
            let (first, last) = full_header_span(area, col, Axis::Col);
            HeaderContextMenu::Column {
                col: first,
                count: last - first + 1,
            }
        })),
        Some(HitTest::RowHeader(row)) => Some(model.with_value(|m| {
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
