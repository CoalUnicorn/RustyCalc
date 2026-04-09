//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

use leptos::prelude::WithValue;

use crate::coord::{CellArea, SheetArea};
use crate::events::{ContentEvent, Location, SpreadsheetEvent, StructureEvent};
use crate::input::{
    error::StructError,
    helpers::{try_mutate, EvaluationMode},
};
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
            let sheet_area = model.with_value(SheetArea::from_view);

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.range_clear_contents(&SheetArea::from_view(m).to_ironcalc_area())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet_area,
            }));
        }
        StructAction::ClearAll => {
            let sheet_area = model.with_value(SheetArea::from_view);

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.range_clear_all(&SheetArea::from_view(m).to_ironcalc_area())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Content(ContentEvent::RangeChanged {
                sheet_area,
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
            let loc = model.with_value(|m| {
                let area = CellArea::from_model(m).normalized();
                Location::new(m.get_selected_sheet(), area.r1, area.height())
            });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_model(m).normalized();
                    m.insert_rows(m.get_selected_sheet(), area.r1, area.height())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_inserted(
                loc,
            )));
        }
        StructAction::InsertColumns => {
            let loc = model.with_value(|m| {
                let area = CellArea::from_model(m).normalized();
                Location::new(m.get_selected_sheet(), area.c1, area.width())
            });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_model(m).normalized();
                    m.insert_columns(m.get_selected_sheet(), area.c1, area.width())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_inserted(loc),
            ));
        }
        StructAction::DeleteRows => {
            let loc = model.with_value(|m| {
                let area = CellArea::from_model(m).normalized();
                Location::new(m.get_selected_sheet(), area.r1, area.height())
            });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_model(m).normalized();
                    m.delete_rows(m.get_selected_sheet(), area.r1, area.height())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_deleted(
                loc,
            )));
        }
        StructAction::DeleteColumns => {
            let loc = model.with_value(|m| {
                let area = CellArea::from_model(m).normalized();
                Location::new(m.get_selected_sheet(), area.c1, area.width())
            });

            try_mutate(
                model,
                state,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_model(m).normalized();
                    m.delete_columns(m.get_selected_sheet(), area.c1, area.width())
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
