//! `handle_dblclick`: auto-fit on resize-seam double-click, start edit on cell double-click.

use leptos::prelude::*;

use crate::coord::{CellAddress, CellArea};
use crate::input::keyboard::{SpreadsheetAction, execute};
use crate::input::structure::StructAction;
use crate::model::{ActiveCellQuery, FormulaAnalyzer};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use iron_canvas_core::Point;
use iron_canvas_core::scene_geometry::{GridHit, GridResize};

use super::cursor_hint::{HIT_ZONE, autofill_hit};
use super::header_span::{Axis, full_header_span};
use super::{CanvasHandle, with_canvas};

pub fn handle_dblclick(
    ev: web_sys::MouseEvent,
    model: ModelStore,
    state: WorkbookState,
    icv: CanvasHandle,
) {
    let point = Point {
        x: ev.offset_x(),
        y: ev.offset_y(),
    };

    if let Some(target) = with_canvas(icv, |h| h.resize_target(point, HIT_ZONE)).flatten() {
        // Excel-style auto-fit: scan the whole used range (not just the
        // painted viewport), and when the boundary sits inside a full-header
        // multi-selection, fit every selected column/row to its OWN content.
        // `full_header_span` collapses to `(idx, idx)` for a lone header, so
        // the single-target case falls out of the same loop. Each column is a
        // separate `set_columns_width` call — hence a separate undo step —
        // because ironcalc groups undo per call (`push_diff_list`).
        let (dim, area) = model.with_value(|m| (m.sheet_dimension(), CellArea::from_view(m)));
        match target {
            GridResize::Column(col) => {
                let (first, last) = full_header_span(area, col, Axis::Col);
                for c in first..=last {
                    // A failed measurement (no model, unreadable sheet or
                    // column extent) is not "no content": skip that column.
                    let measured = model
                        .with_value(|m| {
                            icv.with_value(|slot| {
                                slot.as_ref()
                                    .and_then(|h| h.fit_column_width(m, c, dim.r1, dim.r2).ok())
                            })
                        })
                        .flatten();
                    if let Some(w) = measured {
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
            GridResize::Row(row) => {
                let (first, last) = full_header_span(area, row, Axis::Row);
                for r in first..=last {
                    // A failed measurement (no model, unreadable sheet or
                    // row extent) is not "no content": skip that row.
                    let measured = model
                        .with_value(|m| {
                            icv.with_value(|slot| {
                                slot.as_ref()
                                    .and_then(|h| h.fit_row_height(m, r, dim.c1, dim.c2).ok())
                            })
                        })
                        .flatten();
                    if let Some(h) = measured {
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

    let cell = matches!(
        with_canvas(icv, |h| h.hit_test(point)).flatten(),
        Some(GridHit::Cell(_))
    );
    let near_handle = with_canvas(icv, |h| autofill_hit(h, model, point)).unwrap_or(false);
    if !(cell || near_handle) {
        return;
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
