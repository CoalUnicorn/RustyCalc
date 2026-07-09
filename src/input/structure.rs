//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

use leptos::prelude::WithValue;

use crate::coord::{CellArea, SheetRange};
use crate::events::{ContentEvent, FormatEvent, Location, SpreadsheetEvent, StructureEvent};
use crate::input::error::StructError;
use crate::model::{EvaluationMode, try_mutate};
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
    /// Insert 1 row at a specific position.
    InsertRowAt {
        row: i32,
    },
    /// Insert 1 column at a specific position.
    InsertColumnAt {
        col: i32,
    },
    /// Delete 1 row at a specific position.
    DeleteRowAt {
        row: i32,
    },
    /// Delete 1 column at a specific position.
    DeleteColumnAt {
        col: i32,
    },
    /// Move a column by delta positions (-1 = left, +1 = right).
    MoveColumn {
        col: i32,
        delta: i32,
    },
    /// Move a row by delta positions (-1 = up, +1 = down).
    MoveRow {
        row: i32,
        delta: i32,
    },
    /// Set a uniform width across `count` columns starting at `col`.
    SetColumnWidth {
        col: i32,
        count: i32,
        width: f64,
    },
    /// Set a uniform height across `count` rows starting at `row`.
    SetRowHeight {
        row: i32,
        count: i32,
        height: f64,
    },
    /// Freeze columns from the left up to and including `col`.
    FreezeUpToColumn {
        col: i32,
    },
    /// Freeze rows from the top up to and including `row`.
    FreezeUpToRow {
        row: i32,
    },
    /// Remove all frozen panes.
    Unfreeze,
}

/// Dispatch a [`StructAction`] against the model and UI state.
pub fn execute_struct(
    action: &StructAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), StructError> {
    match action {
        StructAction::Delete => {
            let sheet_area = model.with_value(SheetRange::from_view);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.range_clear_contents(&SheetRange::from_view(m).to_ironcalc_area())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_events([
                SpreadsheetEvent::Content(ContentEvent::RangeChanged { sheet_area }),
                SpreadsheetEvent::Content(ContentEvent::CalculationUpdated {
                    affected_sheets: vec![sheet_area.sheet],
                }),
            ]);
        }
        StructAction::ClearAll => {
            let sheet_area = model.with_value(SheetRange::from_view);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.range_clear_all(&SheetRange::from_view(m).to_ironcalc_area())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_events([
                SpreadsheetEvent::Content(ContentEvent::RangeChanged { sheet_area }),
                SpreadsheetEvent::Content(ContentEvent::CalculationUpdated {
                    affected_sheets: vec![sheet_area.sheet],
                }),
            ]);
        }
        StructAction::Undo => {
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.undo().map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));
        }
        StructAction::Redo => {
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.redo().map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;
            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::DocumentReset));
        }
        StructAction::InsertRows => {
            let loc = model.with_value(|m| {
                let area = CellArea::from_view(m).normalized();
                Location::new(m.get_selected_sheet(), area.r1, area.height())
            });

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_view(m).normalized();
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
                let area = CellArea::from_view(m).normalized();
                Location::new(m.get_selected_sheet(), area.c1, area.width())
            });

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_view(m).normalized();
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
                let area = CellArea::from_view(m).normalized();
                Location::new(m.get_selected_sheet(), area.r1, area.height())
            });

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_view(m).normalized();
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
                let area = CellArea::from_view(m).normalized();
                Location::new(m.get_selected_sheet(), area.c1, area.width())
            });

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    let area = CellArea::from_view(m).normalized();
                    m.delete_columns(m.get_selected_sheet(), area.c1, area.width())
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_deleted(loc),
            ));
        }

        // Targeted operations (context menu)
        StructAction::InsertRowAt { row } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let loc = Location::new(sheet, *row, 1);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.insert_rows(m.get_selected_sheet(), *row, 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_inserted(
                loc,
            )));
        }
        StructAction::InsertColumnAt { col } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let loc = Location::new(sheet, *col, 1);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.insert_columns(m.get_selected_sheet(), *col, 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_inserted(loc),
            ));
        }
        StructAction::DeleteRowAt { row } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let loc = Location::new(sheet, *row, 1);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.delete_rows(m.get_selected_sheet(), *row, 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_deleted(
                loc,
            )));
        }
        StructAction::DeleteColumnAt { col } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let loc = Location::new(sheet, *col, 1);

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.delete_columns(m.get_selected_sheet(), *col, 1)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::columns_deleted(loc),
            ));
        }
        StructAction::MoveColumn { col, delta } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.move_columns_action(m.get_selected_sheet(), *col, 1, *delta)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::ColumnMoved {
                sheet,
                from_col: *col,
                to_col: *col + *delta,
            }));
        }
        StructAction::MoveRow { row, delta } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());

            try_mutate(
                model,
                EvaluationMode::Immediate,
                |m| -> Result<(), StructError> {
                    m.move_rows_action(m.get_selected_sheet(), *row, 1, *delta)
                        .map_err(StructError::Engine)?;
                    Ok(())
                },
            )?;

            state.emit_event(SpreadsheetEvent::Structure(StructureEvent::RowMoved {
                sheet,
                from_row: *row,
                to_row: *row + *delta,
            }));
        }
        StructAction::SetColumnWidth { col, count, width } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let last = col + count - 1;
            let w = width.max(5.0);

            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.set_columns_width(m.get_selected_sheet(), *col, last, w)
                        .map_err(StructError::Engine)
                },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: Some(*col),
                row: None,
            }));
        }
        StructAction::SetRowHeight { row, count, height } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            let last = row + count - 1;
            let h = height.max(3.0);

            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.set_rows_height(m.get_selected_sheet(), *row, last, h)
                        .map_err(StructError::Engine)
                },
            )?;

            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: None,
                row: Some(*row),
            }));
        }
        StructAction::FreezeUpToColumn { col } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.set_frozen_columns_count(m.get_selected_sheet(), *col)
                        .map_err(StructError::Engine)
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: Some(*col),
                row: None,
            }));
        }
        StructAction::FreezeUpToRow { row } => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    m.set_frozen_rows_count(m.get_selected_sheet(), *row)
                        .map_err(StructError::Engine)
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: None,
                row: Some(*row),
            }));
        }
        StructAction::Unfreeze => {
            let sheet = model.with_value(|m| m.get_selected_sheet());
            try_mutate(
                model,
                EvaluationMode::Deferred,
                |m| -> Result<(), StructError> {
                    let s = m.get_selected_sheet();
                    m.set_frozen_rows_count(s, 0).map_err(StructError::Engine)?;
                    m.set_frozen_columns_count(s, 0)
                        .map_err(StructError::Engine)
                },
            )?;
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet,
                col: None,
                row: None,
            }));
        }
    }
    Ok(())
}
