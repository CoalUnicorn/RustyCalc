//! `handle_dblclick`: start edit on cell double-click.
//!
//! Header / corner double-clicks fall through (not yet wired).

use leptos::prelude::*;

use crate::coord::CellAddress;
use crate::model::{FormulaAnalyzer, SheetQuery};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use iron_canvas_core::types::ui::HitTest;

use super::{CanvasHandle, with_canvas};

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
            address: CellAddress {
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
