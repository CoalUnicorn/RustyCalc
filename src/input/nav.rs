//! Navigation actions: arrow keys, page up/down, home/end, sheet switching.

use leptos::prelude::WithValue;

use crate::events::{NavigationEvent, SpreadsheetEvent};
use crate::input::error::NavError;
use crate::input::helpers::{mutate, try_mutate, EvaluationMode};
use crate::model::{ArrowKey, FrontendModel, PageDir};
use crate::state::{ModelStore, WorkbookState};

/// Helper to emit SelectionChanged event after navigation
fn emit_selection_changed(model: ModelStore, state: &WorkbookState) {
    let address = model.with_value(|m: &ironcalc_base::UserModel<'static>| {
        let v = m.get_selected_view();
        crate::model::CellAddress {
            sheet: v.sheet,
            row: v.row,
            column: v.column,
        }
    });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionChanged { address },
    ));
}

/// Helper to emit SelectionRangeChanged event after range operations
fn emit_selection_range_changed(model: ModelStore, state: &WorkbookState) {
    let (sheet, start_row, start_col, end_row, end_col) =
        model.with_value(|m: &ironcalc_base::UserModel<'static>| {
            let v = m.get_selected_view();
            let [r1, c1, r2, c2] = v.range;
            (v.sheet, r1, c1, r2, c2)
        });
    state.emit_event(SpreadsheetEvent::Navigation(
        NavigationEvent::SelectionRangeChanged {
            sheet,
            start_row,
            start_col,
            end_row,
            end_col,
        },
    ));
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavAction {
    /// Move the active cell one step in a direction.
    Arrow(ArrowKey),
    /// Ctrl+Arrow: jump to the data boundary in a direction.
    Edge(ArrowKey),
    /// Ctrl+Home: jump to A1.
    JumpToA1,
    /// Ctrl+End: jump to the last used cell.
    JumpToLastCell,
    /// Shift+Arrow: extend the selection range.
    ExpandSelection(ArrowKey),
    PageDown,
    PageUp,
    /// Home: move to column A of the current row.
    RowHome,
    /// End: move to the last used cell in the current row.
    RowEnd,
    /// Alt+Arrow: cycle sheets; +1 = next, -1 = previous.
    SwitchSheet(i32),
    /// Ctrl+A: select the used data range.
    SelectAll,
}

pub fn execute_nav(
    action: &NavAction,
    model: ModelStore,
    state: &WorkbookState,
) -> Result<(), NavError> {
    match action {
        NavAction::Arrow(dir) => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_arrow(*dir)
            });
            emit_selection_changed(model, state);
        }
        NavAction::Edge(dir) => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_to_edge(*dir)
            });
            emit_selection_changed(model, state);
        }
        NavAction::JumpToA1 => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_set_cell(1, 1)
            });
            emit_selection_changed(model, state);
        }
        NavAction::JumpToLastCell => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_to_edge(ArrowKey::Down);
                m.nav_to_edge(ArrowKey::Right);
            });
            emit_selection_changed(model, state);
        }
        NavAction::ExpandSelection(dir) => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_expand_selection(*dir)
            });
            emit_selection_range_changed(model, state);
        }
        NavAction::PageDown => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_page(PageDir::Down)
            });
            emit_selection_changed(model, state);
        }
        NavAction::PageUp => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_page(PageDir::Up)
            });
            emit_selection_changed(model, state);
        }
        NavAction::RowHome => {
            mutate(model, state, EvaluationMode::Deferred, |m| m.nav_home_row());
            emit_selection_changed(model, state);
        }
        NavAction::RowEnd => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                m.nav_to_edge(ArrowKey::Right)
            });
            emit_selection_changed(model, state);
        }
        NavAction::SwitchSheet(delta) => {
            let delta = *delta;
            let previous_sheet = model
                .with_value(|m: &ironcalc_base::UserModel<'static>| m.get_selected_view().sheet);

            try_mutate(
                model,
                state,
                EvaluationMode::Deferred,
                move |m| -> Result<(), NavError> {
                    let current = m.get_selected_view().sheet;
                    let visible: Vec<u32> = m
                        .get_worksheets_properties()
                        .iter()
                        .filter(|s| s.state == "visible")
                        .map(|s| s.sheet_id)
                        .collect();
                    if let Some(pos) = visible.iter().position(|&id| id == current) {
                        let next = (pos as i32 + delta).rem_euclid(visible.len() as i32) as usize;
                        m.set_selected_sheet(visible[next])
                            .map_err(|e| NavError::Engine(e.to_string()))?;
                    }
                    Ok(())
                },
            )?;

            let new_sheet = model
                .with_value(|m: &ironcalc_base::UserModel<'static>| m.get_selected_view().sheet);
            if previous_sheet != new_sheet {
                state.emit_event(SpreadsheetEvent::Navigation(
                    NavigationEvent::ActiveSheetChanged {
                        from_sheet: previous_sheet,
                        to_sheet: new_sheet,
                    },
                ));
            }
        }
        NavAction::SelectAll => {
            mutate(model, state, EvaluationMode::Deferred, |m| {
                let d = m.sheet_dimension();
                m.nav_select_range(d.min_row, d.min_column, d.max_row, d.max_column);
            });
            emit_selection_range_changed(model, state);
        }
    }
    Ok(())
}
