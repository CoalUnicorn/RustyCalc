//! `SpreadsheetAction` → category-module dispatch.
//!
//! Each category (`nav`, `edit`, `format`, `structure`) returns its own
//! `Result`; this layer maps every error to a `StatusMessage` so the status
//! bar has a single source of truth.

use crate::input::{
    edit::execute_edit, format::execute_format, nav::execute_nav, structure::execute_struct,
};
use crate::state::{ModelStore, StatusMessage, WorkbookState};

use crate::input::action::SpreadsheetAction;

/// Apply a `SpreadsheetAction` to the model and reactive state.
///
/// Dispatches to category-specific execute functions. Clipboard actions
/// are no-ops here - they require the `AppClipboard` store and async OS
/// clipboard APIs, so the Workbook component handles them directly.
pub fn execute(action: &SpreadsheetAction, model: ModelStore, state: &WorkbookState) {
    // Each category returns its own Result type; map to String for the single log point.
    let result: Result<(), String> = match action {
        SpreadsheetAction::Nav(a) => execute_nav(a, model, state).map_err(|e| e.to_string()),
        SpreadsheetAction::Edit(a) => execute_edit(a, model, state).map_err(|e| e.to_string()),
        SpreadsheetAction::Format(a) => execute_format(a, model, state).map_err(|e| e.to_string()),
        SpreadsheetAction::Structure(a) => {
            execute_struct(a, model, state).map_err(|e| e.to_string())
        }
        SpreadsheetAction::Copy | SpreadsheetAction::Cut | SpreadsheetAction::Paste => Ok(()),
    };
    match result {
        Ok(()) => state.status.set(None),
        Err(msg) => state.status.set(Some(StatusMessage::Error(msg))),
    }
}
