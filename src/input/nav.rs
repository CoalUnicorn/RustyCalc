//! Navigation actions: arrow keys, page up/down, home/end, sheet switching.

use crate::input::helpers::{mutate, Recalc};
use crate::model::{ArrowKey, FrontendModel, PageDir};
use crate::state::{ModelStore, WorkbookState};
use crate::util::warn_if_err;

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

pub fn execute_nav(action: &NavAction, model: ModelStore, state: &WorkbookState) {
    match action {
        NavAction::Arrow(dir) => {
            mutate(model, state, Recalc::No, |m| m.nav_arrow(*dir));
        }
        NavAction::Edge(dir) => {
            mutate(model, state, Recalc::No, |m| m.nav_to_edge(*dir));
        }
        NavAction::JumpToA1 => {
            mutate(model, state, Recalc::No, |m| m.nav_set_cell(1, 1));
        }
        NavAction::JumpToLastCell => {
            mutate(model, state, Recalc::No, |m| {
                m.nav_to_edge(ArrowKey::Down);
                m.nav_to_edge(ArrowKey::Right);
            });
        }
        NavAction::ExpandSelection(dir) => {
            mutate(model, state, Recalc::No, |m| m.nav_expand_selection(*dir));
        }
        NavAction::PageDown => {
            mutate(model, state, Recalc::No, |m| m.nav_page(PageDir::Down));
        }
        NavAction::PageUp => {
            mutate(model, state, Recalc::No, |m| m.nav_page(PageDir::Up));
        }
        NavAction::RowHome => {
            mutate(model, state, Recalc::No, |m| m.nav_home_row());
        }
        NavAction::RowEnd => {
            mutate(model, state, Recalc::No, |m| m.nav_to_edge(ArrowKey::Right));
        }
        NavAction::SwitchSheet(delta) => {
            let delta = *delta;
            mutate(model, state, Recalc::No, move |m| {
                let current = m.get_selected_view().sheet;
                let visible: Vec<u32> = m
                    .get_worksheets_properties()
                    .iter()
                    .filter(|s| s.state == "visible")
                    .map(|s| s.sheet_id)
                    .collect();
                if let Some(pos) = visible.iter().position(|&id| id == current) {
                    let next = (pos as i32 + delta).rem_euclid(visible.len() as i32) as usize;
                    warn_if_err(m.set_selected_sheet(visible[next]), "set_selected_sheet");
                }
            });
        }
        NavAction::SelectAll => {
            mutate(model, state, Recalc::No, |m| {
                let d = m.sheet_dimension();
                m.nav_select_range(d.min_row, d.min_column, d.max_row, d.max_column);
            });
        }
    }
}
