//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

use leptos::prelude::WithValue;

use crate::events::{ContentEvent, Location, SpreadsheetEvent, StructureEvent};
use crate::input::error::StructError;
use crate::input::helpers::{make_area, selection_bounds, try_mutate, EvaluationMode};
use crate::state::{ModelStore, WorkbookState};

/// Structural mutations: delete/clear cell content, undo/redo, and row/column insert/delete.
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

/// Dispatch a [`StructAction`] against the model and UI state.
pub fn execute_struct(
    action: &StructAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), StructError> {
    match action {
        StructAction::Delete => {
            let (sheet, start_row, start_col, end_row, end_col) =
                model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    (v.sheet, r1, c1, r2, c2)
                });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2))
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

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

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let [r1, c1, r2, c2] = v.range;
                    m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2))
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet,
                start_row,
                start_col,
                end_row,
                end_col,
            }));
        }
        StructAction::Undo => {
            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.undo().map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        }
        StructAction::Redo => {
            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.redo().map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
        }
        StructAction::InsertRows => {
            let loc = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                Location::new(v.sheet, r_min, r_max - r_min + 1)
            });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let ((r_min, r_max), _) = selection_bounds(v.range);
                    m.insert_rows(v.sheet, r_min, r_max - r_min + 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

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

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let (_, (c_min, c_max)) = selection_bounds(v.range);
                    m.insert_columns(v.sheet, c_min, c_max - c_min + 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

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

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let ((r_min, r_max), _) = selection_bounds(v.range);
                    m.delete_rows(v.sheet, r_min, r_max - r_min + 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

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

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let v = m.get_selected_view();
                    let (_, (c_min, c_max)) = selection_bounds(v.range);
                    m.delete_columns(v.sheet, c_min, c_max - c_min + 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_deleted(loc),
            ));
        }
    }
    Ok(())
}
