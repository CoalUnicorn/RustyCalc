//! `handle_dblclick`: auto-fit on resize-seam double-click, start edit on cell double-click.

use leptos::prelude::*;

use crate::coord::{CellAddress, CellArea};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::structure::StructAction;
use crate::model::{FormulaAnalyzer, SheetQuery};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use iron_canvas_core::types::ui::{HitTest, ResizeTarget};

use super::cursor_hint::HIT_ZONE;
use super::header_span::{Axis, full_header_span};
use super::{CanvasHandle, with_canvas};

pub fn handle_dblclick(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let x = ev.offset_x() as f64;
    let y = ev.offset_y() as f64;

    if let Some(target) = with_canvas(icv, |ic| ic.resize_handle_at(x, y, HIT_ZONE)).flatten() {
        // Excel-style auto-fit: scan the whole used range (not just the
        // painted viewport), and when the boundary sits inside a full-header
        // multi-selection, fit every selected column/row to its OWN content.
        // `full_header_span` collapses to `(idx, idx)` for a lone header, so
        // the single-target case falls out of the same loop. Each column is a
        // separate `set_columns_width` call — hence a separate undo step —
        // because ironcalc groups undo per call (`push_diff_list`).
        let (dim, area) = model.with_value(|m| (m.sheet_dimension(), CellArea::from_view(m)));
        match target {
            ResizeTarget::Column(col) => {
                let (first, last) = full_header_span(area, col, Axis::Col);
                for c in first..=last {
                    if let Some(w) =
                        with_canvas(icv, |ic| ic.fit_column_width(c, dim.r1, dim.r2)).flatten()
                    {
                        execute(
                            &SpreadsheetAction::Structure(StructAction::SetColumnWidth {
                                col: c,
                                count: 1,
                                width: w,
                            }),
                            model,
                            &state,
                        );
                    }
                }
            }
            ResizeTarget::Row(row) => {
                let (first, last) = full_header_span(area, row, Axis::Row);
                for r in first..=last {
                    if let Some(h) =
                        with_canvas(icv, |ic| ic.fit_row_height(r, dim.c1, dim.c2)).flatten()
                    {
                        execute(
                            &SpreadsheetAction::Structure(StructAction::SetRowHeight {
                                row: r,
                                count: 1,
                                height: h,
                            }),
                            model,
                            &state,
                        );
                    }
                }
            }
        }
        ev.prevent_default();
        return;
    }

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
