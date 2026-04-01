//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

use leptos::prelude::WithValue;

use crate::input::helpers::{make_area, mutate, selection_bounds, Eval};
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

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_contents",
                );
            });

            // Fire content changed event for range deletion
            state.emit_event(crate::events::SpreadsheetEvent::Content(
                crate::events::ContentEvent::RangeChanged {
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                },
            ));
        }
        StructAction::ClearAll => {
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    (v.sheet, r1, c1, r2, c2)
                });

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_all",
                );
            });

            // Fire content changed event for range clear all
            state.emit_event(crate::events::SpreadsheetEvent::Content(
                crate::events::ContentEvent::RangeChanged {
                    sheet,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                },
            ));
        }
        StructAction::Undo => {
            mutate(model, state, Eval::No, |m| {
                warn_if_err(m.undo(), "undo");
            });

            // Fire generic change event for undo (affects potentially everything)
            state.emit_event(crate::events::SpreadsheetEvent::Content(
                crate::events::ContentEvent::GenericChange,
            ));
        }
        StructAction::Redo => {
            mutate(model, state, Eval::No, |m| {
                warn_if_err(m.redo(), "redo");
            });

            // Fire generic change event for redo (affects potentially everything)
            state.emit_event(crate::events::SpreadsheetEvent::Content(
                crate::events::ContentEvent::GenericChange,
            ));
        }
        StructAction::InsertRows => {
            let (sheet, start_row, count) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let ((r_min, r_max), _) = selection_bounds(v.range);
                    (v.sheet, r_min, r_max - r_min + 1)
                });

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_rows(v.sheet, r_min, r_max - r_min + 1),
                    "insert_rows",
                );
            });

            // Fire structure changed event for row insertion
            state.emit_event(crate::events::SpreadsheetEvent::Structure(
                crate::events::StructureEvent::StructureChanged(
                    crate::events::StructureChange::insert_rows(sheet, start_row, count),
                ),
            ));
        }
        StructAction::InsertColumns => {
            let (sheet, start_col, count) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let (_, (c_min, c_max)) = selection_bounds(v.range);
                    (v.sheet, c_min, c_max - c_min + 1)
                });

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_columns(v.sheet, c_min, c_max - c_min + 1),
                    "insert_columns",
                );
            });

            // Fire structure changed event for column insertion
            state.emit_event(crate::events::SpreadsheetEvent::Structure(
                crate::events::StructureEvent::StructureChanged(
                    crate::events::StructureChange::insert_columns(sheet, start_col, count),
                ),
            ));
        }
        StructAction::DeleteRows => {
            let (sheet, start_row, count) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let ((r_min, r_max), _) = selection_bounds(v.range);
                    (v.sheet, r_min, r_max - r_min + 1)
                });

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_rows(v.sheet, r_min, r_max - r_min + 1),
                    "delete_rows",
                );
            });

            // Fire structure changed event for row deletion
            state.emit_event(crate::events::SpreadsheetEvent::Structure(
                crate::events::StructureEvent::StructureChanged(
                    crate::events::StructureChange::delete_rows(sheet, start_row, count),
                ),
            ));
        }
        StructAction::DeleteColumns => {
            let (sheet, start_col, count) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let (_, (c_min, c_max)) = selection_bounds(v.range);
                    (v.sheet, c_min, c_max - c_min + 1)
                });

            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_columns(v.sheet, c_min, c_max - c_min + 1),
                    "delete_columns",
                );
            });

            // Fire structure changed event for column deletion
            state.emit_event(crate::events::SpreadsheetEvent::Structure(
                crate::events::StructureEvent::StructureChanged(
                    crate::events::StructureChange::delete_columns(sheet, start_col, count),
                ),
            ));
        }
    }
}
