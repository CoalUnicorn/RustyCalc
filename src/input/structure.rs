//! Structural mutations: delete, clear, undo/redo, insert/delete rows/columns.

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
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_contents",
                );
            });
        }
        StructAction::ClearAll => {
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_all",
                );
            });
        }
        StructAction::Undo => {
            mutate(model, state, Eval::No, |m| {
                warn_if_err(m.undo(), "undo");
            });
        }
        StructAction::Redo => {
            mutate(model, state, Eval::No, |m| {
                warn_if_err(m.redo(), "redo");
            });
        }
        StructAction::InsertRows => {
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_rows(v.sheet, r_min, r_max - r_min + 1),
                    "insert_rows",
                );
            });
        }
        StructAction::InsertColumns => {
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_columns(v.sheet, c_min, c_max - c_min + 1),
                    "insert_columns",
                );
            });
        }
        StructAction::DeleteRows => {
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_rows(v.sheet, r_min, r_max - r_min + 1),
                    "delete_rows",
                );
            });
        }
        StructAction::DeleteColumns => {
            mutate(model, state, Eval::Yes, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_columns(v.sheet, c_min, c_max - c_min + 1),
                    "delete_columns",
                );
            });
        }
    }
}
