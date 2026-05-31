//! `handle_mousedown`: hit-test → drag start or click dispatch.
//!
//! Resize handles are probed first (they straddle the header/cell seam
//! by `HIT_ZONE` px), then the normal hit-test routes to the four click
//! helpers in `click.rs` or starts a formula-reference drag.

use crate::coord::CellArea;
use crate::state::{DragState, ModelStore, WorkbookState};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};
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
            ResizeTarget::Column(col) => {
                let span = full_header_span(area, col, Axis::Col);
                state.drag.set(DragState::ResizingCol { col, span, x });
            }
            ResizeTarget::Row(row) => {
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
        HitTest::ColHeader(col) => handle_col_header_click(&ev, col, model, state),
        HitTest::RowHeader(row) => handle_row_header_click(&ev, row, model, state),
        HitTest::AutofillHandle { row, column } => {
            handle_cell_click(&ev, row, column, true, model, state)
        }
        HitTest::Cell { row, column } => handle_cell_click(&ev, row, column, false, model, state),
        HitTest::FormulaRef {
            ref_idx,
            zone,
            grab_row,
            grab_col,
        } => {
            handle_formula_ref_mousedown(&ev, ref_idx, zone, grab_row, grab_col, model, state);
        }
        HitTest::Outside => {}
    }
}
