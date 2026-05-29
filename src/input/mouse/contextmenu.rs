//! `handle_contextmenu`: right-click on header → show context menu.
//!
//! Clicks in the cell grid are ignored — cell context menu not yet
//! implemented.

use leptos::prelude::*;

use crate::coord::CellArea;
use crate::state::{ContextMenuState, HeaderContextMenu, ModelStore, WorkbookState};
use iron_canvas_core::geometry::constants::{LAST_COLUMN, LAST_ROW};
use iron_canvas_core::types::ui::HitTest;

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
            let area = CellArea::from_view(m).normalized();
            // Multi-column selection if the clicked col is inside a full-column range.
            let (col, count) = if area.r2 >= LAST_ROW && area.c1 <= col && col <= area.c2 {
                (area.c1, area.c2 - area.c1 + 1)
            } else {
                (col, 1)
            };
            HeaderContextMenu::Column { col, count }
        })),
        Some(HitTest::RowHeader(row)) => Some(model.with_value(|m| {
            let area = CellArea::from_view(m).normalized();
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
