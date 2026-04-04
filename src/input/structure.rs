//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

use leptos::prelude::WithValue;

use crate::events::{ContentEvent, Location, SpreadsheetEvent, StructureEvent};
use crate::input::helpers::{make_area, mutate, selection_bounds, EvaluationMode};
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

#[derive(Debug, Clone, PartialEq)]
pub enum StructAction {
    /// Delete key: clear cell contents, preserve formatting.
    Delete,
    /// Ctrl+Shift+Delete: clear both contents and formatting.
    ClearAll,
    Undo,
    Redo,
    InsertRows,
    InsertColumns,
    DeleteRows,
    DeleteColumns,
}

pub fn execute_struct(action: &StructAction, model: ModelStore, state: &WorkbookState) {
    match action {
        StructAction::Delete => {
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    (v.sheet, r1, c1, r2, c2)
                });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_contents",
                );
            });

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        StructAction::ClearAll => {
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    (v.sheet, r1, c1, r2, c2)
                });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_all",
                );
            });

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        StructAction::Undo => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                warn_if_err(m.undo(), "undo");
            });

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        }
        StructAction::Redo => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                warn_if_err(m.redo(), "redo");
            });

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        }
        StructAction::InsertRows => {
            let loc = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                Location::new(v.sheet, r_min, r_max - r_min + 1)
            });

            // Can we use this ?
            // let loc2 = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
            //     let v = m.get_selected_view();
            //     Location::from_selecton_bounds(
            //         v.sheet,
            //         Origin::Row { start: None },
            //         selection_bounds(v.range),
            //     )
            // });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);

                warn_if_err(
                    // m.insert_rows(Location::for_insert(
                    //     v.sheet,
                    //     Dimension::Row { start: None },
                    //     selection_bounds(v.range),
                    // ))
                    m.insert_rows(v.sheet, r_min, r_max - r_min + 1),
                    "insert_rows",
                );
            });

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_inserted(
                loc,
            )));
        }
        StructAction::InsertColumns => {
            let loc = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                Location::new(v.sheet, c_min, c_max - c_min + 1)
            });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_columns(v.sheet, c_min, c_max - c_min + 1),
                    "insert_columns",
                );
            });

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_inserted(loc),
            ));
        }
        StructAction::DeleteRows => {
            let loc = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                Location::new(v.sheet, r_min, r_max - r_min + 1)
            });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_rows(v.sheet, r_min, r_max - r_min + 1),
                    "delete_rows",
                );
            });

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_deleted(
                loc,
            )));
        }
        StructAction::DeleteColumns => {
            let loc = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                Location::new(v.sheet, c_min, c_max - c_min + 1)
            });

            mutate(model, state, EvaluationMode::Immediate, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_columns(v.sheet, c_min, c_max - c_min + 1),
                    "delete_columns",
                );
            });

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_deleted(loc),
            ));
        }
    }
}
