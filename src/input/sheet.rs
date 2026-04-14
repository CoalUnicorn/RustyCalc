//! Sheet-level actions: select, add, delete, hide, unhide, rename, set color.
//!
//! Called directly from components — not routed through the keyboard
//! `classify_key`/`execute` pipeline. Persistence is handled centrally
//! by the EventBus-driven auto-save in `app.rs`.
//!
//! Follows the `WorkbookAction` pattern in `workbook.rs`.

use leptos::prelude::WithValue;

use crate::events::{FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent};
use crate::input::error::SheetError;
use crate::model::{try_mutate, EvaluationMode, FrontendModel};
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

/// Sheet-level operations on the current workbook.
///
/// Separate from `StructAction` because these involve state coordination
/// beyond what the keyboard action pipeline handles.
/// Callers are responsible for confirmation dialogs before `Delete`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SheetAction {
    /// Switch to a sheet by index.
    Select(u32),
    /// Add a new sheet at the end.
    Add,
    /// Delete a sheet (caller confirms first).
    Delete(u32),
    /// Hide a visible sheet.
    Hide(u32),
    /// Unhide a hidden sheet and select it.
    Unhide(u32),
    /// Rename a sheet.
    Rename { sheet: u32, name: String },
    /// Set or clear the tab color.
    SetColor { sheet: u32, color: Option<String> },
    /// Duplicate a sheet within the same workbook.
    Duplicate(u32),
    /// Reorder a sheet tab to a new position.
    Move { sheet: u32, to_index: u32 },
}

/// Execute a [`SheetAction`] against the model and emit the appropriate events.
pub fn execute_sheet(action: &SheetAction, model: ModelStore, state: &WorkbookState) {
    match action {
        SheetAction::Select(sheet_idx) => {
            let previous = model.with_value(|m| m.get_selected_view().sheet);
            if previous == *sheet_idx {
                return;
            }
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.set_selected_sheet(*sheet_idx).map_err(SheetError::Engine)
                }),
                "set_selected_sheet",
            );
            state.emit_event(SpreadsheetEvent::Navigation(
                NavigationEvent::ActiveSheetChanged {
                    from_sheet: previous,
                    to_sheet: *sheet_idx,
                },
            ));
        }

        SheetAction::Add => {
            let sheet_idx = model.with_value(|m| m.get_worksheets_properties().len() as u32);
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.new_sheet().map_err(SheetError::Engine)
                }),
                "new_sheet",
            );
            let name = model.with_value(|m| m.get_sheet_name(sheet_idx as usize));

            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetAdded {
                    sheet: sheet_idx,
                    name,
                },
            ));
        }

        SheetAction::Delete(sheet_idx) => {
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.delete_sheet(*sheet_idx).map_err(SheetError::Engine)
                }),
                "delete_sheet",
            );
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetDeleted { sheet: *sheet_idx },
            ));
        }

        SheetAction::Hide(sheet_idx) => {
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.hide_sheet(*sheet_idx).map_err(SheetError::Engine)
                }),
                "hide_sheet",
            );
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetHidden { sheet: *sheet_idx },
            ));
        }

        SheetAction::Unhide(sheet_idx) => {
            let name = model.with_value(|m| m.get_sheet_name(*sheet_idx as usize));
            let previous = model.with_value(|m| m.get_selected_view().sheet);
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.unhide_sheet(*sheet_idx).map_err(SheetError::Engine)
                }),
                "unhide_sheet",
            );
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.set_selected_sheet(*sheet_idx).map_err(SheetError::Engine)
                }),
                "set_selected_sheet",
            );
            state.emit_events([
                SpreadsheetEvent::Structure(StructureEvent::WorksheetUnhidden {
                    sheet: *sheet_idx,
                    name,
                }),
                SpreadsheetEvent::Navigation(NavigationEvent::ActiveSheetChanged {
                    from_sheet: previous,
                    to_sheet: *sheet_idx,
                }),
            ]);
        }

        SheetAction::Rename { sheet, name } => {
            let old_name = model.with_value(|m| m.get_sheet_name(*sheet as usize));

            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.rename_sheet(*sheet, name).map_err(SheetError::Engine)
                }),
                "rename_sheet",
            );
            state.emit_event(SpreadsheetEvent::Structure(
                StructureEvent::WorksheetRenamed {
                    sheet: *sheet,
                    old_name,
                    new_name: name.clone(),
                },
            ));
        }

        SheetAction::SetColor { sheet, color } => {
            let hex = color.as_deref().unwrap_or("");
            warn_if_err(
                try_mutate(model, EvaluationMode::Deferred, |m| {
                    m.set_sheet_color(*sheet, hex).map_err(SheetError::Engine)
                }),
                "set_sheet_color",
            );
            if !hex.is_empty() {
                state.add_recent_color(hex);
            }
            state.emit_event(SpreadsheetEvent::Format(FormatEvent::LayoutChanged {
                sheet: *sheet,
                col: None,
                row: None,
            }));
        }

        SheetAction::Duplicate(_) => {
            todo!("SheetAction::Duplicate not yet implemented")
        }
        SheetAction::Move { .. } => {
            todo!("SheetAction::Move not yet implemented")
        }
    }
}
