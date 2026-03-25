// ==============================================================================
// SpreadsheetAction — typed description of every user-triggered mutation.
//
// Two public entry points:
//   classify_key(...)  →  pure mapping from a key event to an action
//   execute(...)       →  applies the action: model mutation, signal updates,
//                         formula evaluation, persistence, focus management
// ==============================================================================
//
// ── HOW TO ADD A NEW KEY ACTION ───────────────────────────────────────────────
//
// Three steps, always in the same order:
//
//   1. Add a variant to `SpreadsheetAction`.
//      Name it after the user's *intent*, not the key.
//      Good: `DuplicateRow`    Bad: `CtrlD`
//
//   2. Add a branch to `classify_key`.
//      Pick the right modifier block (ctrl-only / shift-only / plain / …) and
//      map the key string to the new variant.
//      Example — Ctrl+D duplicates the current row:
//
//          // inside the `ctrl && !shift && !alt` block:
//          "d" => return Some(DuplicateRow),
//
//   3. Add an arm to the `match` in `execute`.
//      Call `mutate(model, state, Evaluate::Yes, |m| { … })` for model
//      mutations.  Pass `Evaluate::Yes` whenever formula results may change
//      (cell writes, row/column inserts/deletes, undo/redo of those).
//      Example:
//
//          SpreadsheetAction::DuplicateRow => {
//              mutate(model, state, Evaluate::Yes, |m| {
//                  let v = m.get_selected_view();
//                  m.insert_rows(v.sheet, v.row + 1, 1).ok();
//                  // copy logic …
//              });
//          }
//
// ── MODIFYING AN EXISTING ACTION ──────────────────────────────────────────────
//
//   • To change the key binding:  edit only `classify_key`.
//   • To change what it does:     edit only `execute`.
//   • To change the name:         rename the variant and update both.
//
//   `classify_key` is a pure function — it must never touch the DOM, signals,
//   or the model.  All side-effects belong in `execute`.
//
// ── NOTE ON POINT-MODE ARROWS ─────────────────────────────────────────────────
//
//   Arrow keys while editing a formula can extend a cell-reference range
//   instead of committing the edit.  That logic runs as a *pre-check* in the
//   `on_keydown` closure in `workbook.rs`, before `classify_key` is called,
//   because it requires reading the textarea cursor position from the DOM.
//   Do not try to move it into `classify_key`.
//
// ── NOTE ON CLIPBOARD ACTIONS ─────────────────────────────────────────────────
//
//   `Copy`, `Cut`, and `Paste` are classified by `classify_key` but NOT
//   executed by `execute`.  They require the `AppClipboard` context store
//   and async OS clipboard APIs, so the Workbook component handles them
//   directly after receiving the classified action.
//
// ==============================================================================

use ironcalc_base::expressions::types::Area;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::model::{ArrowKey, FrontendModel, PageDir};
use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::storage;
use crate::util::warn_if_err;

/// Whether `mutate` should recalculate formulas after applying the closure.
///
/// Pass `Eval::Yes` when the mutation may change formula results
/// (cell writes, row/column inserts/deletes).
/// Pass `Eval::No` for pure navigation or selection changes.
#[derive(Clone, Copy)]
enum Eval {
    Y,
    N,
}

// ── SpreadsheetAction ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SpreadsheetAction {
    // ── Navigation ──────────────────────────────────────────────────────────
    /// Move the active cell one step in a direction.
    Navigate(ArrowKey),
    /// Ctrl+Arrow: jump to the data boundary in a direction.
    NavigateEdge(ArrowKey),
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

    // ── Editing ─────────────────────────────────────────────────────────────
    /// Printable key: start a new edit with this character as the initial text.
    StartEdit(String),
    /// F2: enter edit mode preserving the existing cell content.
    EnterEditMode,
    /// Enter/Tab: write the edit buffer to the model then navigate.
    CommitAndNavigate(ArrowKey),
    /// Escape: discard the edit buffer without writing to the model.
    CancelEdit,

    // ── Clipboard (handled by the Workbook component, not execute) ──────────
    Copy,
    Cut,
    Paste,

    // ── Structural mutations ─────────────────────────────────────────────────
    /// Delete key: clear cell contents, preserve formatting.
    Delete,
    /// Ctrl+Shift+Delete: clear both contents and formatting.
    ClearAll,
    /// Ctrl+A: select the used data range.
    SelectAll,
    Undo,
    Redo,
    InsertRows,
    InsertColumns,
    DeleteRows,
    DeleteColumns,
}

// ── Key classification ────────────────────────────────────────────────────────

/// Map a keyboard event to a `SpreadsheetAction`, or `None` if unhandled.
///
/// This function is pure — no side effects, no DOM access.
///
/// **Point-mode arrow navigation** is excluded: it requires reading the
/// textarea cursor position from the DOM, so it is handled as a pre-check
/// in the keydown closure before this function is called.
pub fn classify_key(
    key: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
    edit: Option<&EditingCell>,
) -> Option<SpreadsheetAction> {
    use ArrowKey::*;
    use SpreadsheetAction::*;

    // ── While editing ────────────────────────────────────────────────────────
    // Arrow keys in Accept mode that reach here have already failed the
    // point-mode pre-check, so they should commit the edit and navigate.
    if let Some(e) = edit {
        return match key {
            "Enter" => Some(CommitAndNavigate(Down)),
            "Tab" if shift => Some(CommitAndNavigate(Left)),
            "Tab" => Some(CommitAndNavigate(Right)),
            "Escape" => Some(CancelEdit),
            "ArrowDown" if e.mode == EditMode::Accept => Some(CommitAndNavigate(Down)),
            "ArrowUp" if e.mode == EditMode::Accept => Some(CommitAndNavigate(Up)),
            "ArrowLeft" if e.mode == EditMode::Accept => Some(CommitAndNavigate(Left)),
            "ArrowRight" if e.mode == EditMode::Accept => Some(CommitAndNavigate(Right)),
            _ => None,
        };
    }

    // ── Not editing ──────────────────────────────────────────────────────────

    // Ctrl-only (no shift, no alt).
    if ctrl && !shift && !alt {
        // Lowercase to handle caps-lock correctly.
        match key.to_lowercase().as_str() {
            "z" => return Some(Undo),
            "y" => return Some(Redo),
            "a" => return Some(SelectAll),
            "c" => return Some(Copy),
            "x" => return Some(Cut),
            "v" => return Some(Paste),
            _ => {}
        }
        return match key {
            "Home" => Some(JumpToA1),
            "End" => Some(JumpToLastCell),
            "ArrowRight" => Some(NavigateEdge(Right)),
            "ArrowLeft" => Some(NavigateEdge(Left)),
            "ArrowUp" => Some(NavigateEdge(Up)),
            "ArrowDown" => Some(NavigateEdge(Down)),
            "-" => Some(DeleteRows),
            _ => None,
        };
    }

    // Ctrl+Alt (no shift): delete columns.
    if ctrl && !shift && alt {
        return match key {
            "-" => Some(DeleteColumns),
            _ => None,
        };
    }

    // Ctrl+Shift (no alt): structural edits.
    if ctrl && shift && !alt {
        return match key {
            "Delete" => Some(ClearAll),
            "=" | "+" => Some(InsertRows),
            _ => None,
        };
    }

    // Ctrl+Shift+Alt: insert columns.
    if ctrl && shift && alt {
        return match key {
            "=" | "+" => Some(InsertColumns),
            _ => None,
        };
    }

    // Alt-only (no ctrl, no shift): sheet navigation.
    if alt && !ctrl && !shift {
        return match key {
            "ArrowDown" => Some(SwitchSheet(1)),
            "ArrowUp" => Some(SwitchSheet(-1)),
            _ => None,
        };
    }

    // Shift-only (no ctrl, no alt): extend selection.
    if shift && !ctrl && !alt {
        return match key {
            "ArrowRight" => Some(ExpandSelection(Right)),
            "ArrowLeft" => Some(ExpandSelection(Left)),
            "ArrowUp" => Some(ExpandSelection(Up)),
            "ArrowDown" => Some(ExpandSelection(Down)),
            // Shift+Tab navigates left but does NOT extend the selection.
            "Tab" => Some(Navigate(Left)),
            _ => None,
        };
    }

    // Any remaining modifier combination is not handled here.
    if ctrl || alt {
        return None;
    }

    // Plain keys — no modifiers.
    match key {
        "ArrowRight" | "Tab" => Some(Navigate(Right)),
        "ArrowLeft" => Some(Navigate(Left)),
        "ArrowDown" | "Enter" => Some(Navigate(Down)),
        "ArrowUp" => Some(Navigate(Up)),
        "PageDown" => Some(PageDown),
        "PageUp" => Some(PageUp),
        "Home" => Some(RowHome),
        "End" => Some(RowEnd),
        "Delete" => Some(Delete),
        "Escape" => Some(CancelEdit),
        "F2" => Some(EnterEditMode),
        k if is_printable(k) => Some(StartEdit(k.to_owned())),
        _ => None,
    }
}

// ── Action execution ──────────────────────────────────────────────────────────

/// Apply a `SpreadsheetAction` to the model and reactive state.
///
/// Centralises all side-effects: model mutation, formula evaluation, signal
/// updates, localStorage persistence, and focus restoration.
///
/// **Clipboard actions** (`Copy`, `Cut`, `Paste`) are no-ops here — they
/// require the `AppClipboard` store and async OS clipboard APIs, so the
/// caller (Workbook component) handles them directly.
pub fn execute(action: &SpreadsheetAction, model: ModelStore, state: &WorkbookState) {
    match action {
        SpreadsheetAction::Navigate(dir) => {
            mutate(model, state, Eval::N, |m| {
                m.nav_arrow(*dir);
            });
        }
        SpreadsheetAction::NavigateEdge(dir) => {
            mutate(model, state, Eval::N, |m| {
                m.nav_to_edge(*dir);
            });
        }
        SpreadsheetAction::JumpToA1 => {
            mutate(model, state, Eval::N, |m| {
                m.nav_set_cell(1, 1);
            });
        }
        SpreadsheetAction::JumpToLastCell => {
            mutate(model, state, Eval::N, |m| {
                m.nav_to_edge(ArrowKey::Down);
                m.nav_to_edge(ArrowKey::Right);
            });
        }
        SpreadsheetAction::ExpandSelection(dir) => {
            mutate(model, state, Eval::N, |m| {
                m.nav_expand_selection(*dir);
            });
        }
        SpreadsheetAction::PageDown => {
            mutate(model, state, Eval::N, |m| {
                m.nav_page(PageDir::Down);
            });
        }
        SpreadsheetAction::PageUp => {
            mutate(model, state, Eval::N, |m| {
                m.nav_page(PageDir::Up);
            });
        }
        SpreadsheetAction::RowHome => {
            mutate(model, state, Eval::N, |m| {
                m.nav_home_row();
            });
        }
        SpreadsheetAction::RowEnd => {
            mutate(model, state, Eval::N, |m| {
                m.nav_to_edge(ArrowKey::Right);
            });
        }
        SpreadsheetAction::SwitchSheet(delta) => {
            let delta = *delta;
            mutate(model, state, Eval::N, move |m| {
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
        SpreadsheetAction::StartEdit(text) => {
            model.with_value(|m| {
                let v = m.get_selected_view();
                state.editing_cell.set(Some(EditingCell {
                    sheet: v.sheet,
                    row: v.row,
                    col: v.column,
                    text: text.clone(),
                    mode: EditMode::Accept,
                    focus: EditFocus::Cell,
                }));
            });
            state.request_redraw();
        }
        SpreadsheetAction::EnterEditMode => {
            model.with_value(|m| {
                let v = m.get_selected_view();
                let text = m
                    .get_cell_content(v.sheet, v.row, v.column)
                    .unwrap_or_default();
                state.editing_cell.set(Some(EditingCell {
                    sheet: v.sheet,
                    row: v.row,
                    col: v.column,
                    text,
                    mode: EditMode::Edit,
                    focus: EditFocus::Cell,
                }));
            });
            state.request_redraw();
        }
        SpreadsheetAction::CommitAndNavigate(dir) => {
            if let Some(edit) = state.editing_cell.get_untracked() {
                // Write the edit buffer to the model and recalculate.
                model.update_value(|m| {
                    warn_if_err(
                        m.set_user_input(edit.sheet, edit.row, edit.col, &edit.text),
                        "set_user_input",
                    );
                    m.evaluate();
                });
                // Clear all edit-related state.
                state.editing_cell.set(None);
                state.point_range.set(None);
                state.point_ref_span.set(None);
                // Persist the committed change immediately.
                if let Some(uuid) = state.current_uuid.get_untracked() {
                    model.with_value(|m| storage::save(&uuid, m));
                }
                // Navigate to the next cell and redraw.
                model.update_value(|m| {
                    m.nav_arrow(*dir);
                });
                state.request_redraw();
                crate::util::refocus_workbook();
            }
        }
        SpreadsheetAction::CancelEdit => {
            state.editing_cell.set(None);
            state.point_range.set(None);
            state.point_ref_span.set(None);
            state.request_redraw();
            crate::util::refocus_workbook();
        }

        // ── Clipboard: handled by the Workbook component ─────────────────
        SpreadsheetAction::Copy | SpreadsheetAction::Cut | SpreadsheetAction::Paste => {}

        SpreadsheetAction::Delete => {
            mutate(model, state, Eval::Y, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_contents",
                );
            });
        }
        SpreadsheetAction::ClearAll => {
            mutate(model, state, Eval::Y, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                warn_if_err(
                    m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2)),
                    "range_clear_all",
                );
            });
        }
        SpreadsheetAction::SelectAll => {
            mutate(model, state, Eval::N, |m| {
                let d = m.sheet_dimension();
                m.nav_select_range(d.min_row, d.min_column, d.max_row, d.max_column);
            });
        }
        SpreadsheetAction::Undo => {
            mutate(model, state, Eval::N, |m| {
                warn_if_err(m.undo(), "undo");
            });
        }
        SpreadsheetAction::Redo => {
            mutate(model, state, Eval::N, |m| {
                warn_if_err(m.redo(), "redo");
            });
        }
        SpreadsheetAction::InsertRows => {
            mutate(model, state, Eval::Y, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_rows(v.sheet, r_min, r_max - r_min + 1),
                    "insert_rows",
                );
            });
        }
        SpreadsheetAction::InsertColumns => {
            mutate(model, state, Eval::Y, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                warn_if_err(
                    m.insert_columns(v.sheet, c_min, c_max - c_min + 1),
                    "insert_columns",
                );
            });
        }
        SpreadsheetAction::DeleteRows => {
            mutate(model, state, Eval::Y, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                warn_if_err(
                    m.delete_rows(v.sheet, r_min, r_max - r_min + 1),
                    "delete_rows",
                );
            });
        }
        SpreadsheetAction::DeleteColumns => {
            mutate(model, state, Eval::Y, |m| {
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

// ── Private helpers ───────────────────────────────────────────────────────────

/// Run `f` on the model, optionally call `evaluate`, then trigger a redraw.
fn mutate(
    model: ModelStore,
    state: &WorkbookState,
    evaluate: Eval,
    f: impl FnOnce(&mut UserModel<'static>),
) {
    model.update_value(|m| {
        f(m);
        if matches!(evaluate, Eval::Y) {
            m.evaluate();
        }
    });
    state.request_redraw();
}

/// Build an `Area` from selection corners, normalising min/max automatically.
fn make_area(sheet: u32, r1: i32, c1: i32, r2: i32, c2: i32) -> Area {
    Area {
        sheet,
        row: r1.min(r2),
        column: c1.min(c2),
        height: (r2 - r1).abs() + 1,
        width: (c2 - c1).abs() + 1,
    }
}

/// Returns `((min_row, max_row), (min_col, max_col))` from a `[r1,c1,r2,c2]` range.
pub fn selection_bounds(range: [i32; 4]) -> ((i32, i32), (i32, i32)) {
    let [r1, c1, r2, c2] = range;
    ((r1.min(r2), r1.max(r2)), (c1.min(c2), c1.max(c2)))
}

/// True for single printable characters that should start a cell edit.
fn is_printable(key: &str) -> bool {
    key.chars().count() == 1 && key.as_bytes()[0] >= 0x20
}

// ── Tests ──────────────────────────────────────────────────────────────────────
//
// Run with:
//   wasm-pack test --headless --firefox   (or --chrome)
//
// All tests use `#[wasm_bindgen_test]` because the crate targets wasm32 only.
// The `run_in_browser` config is required for the execute tests (WorkbookState
// calls `gloo_storage::LocalStorage` which needs a real browser environment).
// Pure classify_key and selection_bounds tests also work fine in the browser.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ArrowKey;
    use crate::state::{EditFocus, EditMode, EditingCell};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn accept_cell() -> EditingCell {
        EditingCell {
            sheet: 1,
            row: 1,
            col: 1,
            text: String::new(),
            mode: EditMode::Accept,
            focus: EditFocus::Cell,
        }
    }

    fn edit_cell() -> EditingCell {
        EditingCell {
            sheet: 1,
            row: 1,
            col: 1,
            text: String::new(),
            mode: EditMode::Edit,
            focus: EditFocus::Cell,
        }
    }

    // ── classify_key: plain keys (not editing) ────────────────────────────────

    #[wasm_bindgen_test]
    fn plain_arrows_navigate() {
        let ck = |k| classify_key(k, false, false, false, None);
        assert_eq!(
            ck("ArrowRight"),
            Some(SpreadsheetAction::Navigate(ArrowKey::Right))
        );
        assert_eq!(
            ck("ArrowLeft"),
            Some(SpreadsheetAction::Navigate(ArrowKey::Left))
        );
        assert_eq!(
            ck("ArrowDown"),
            Some(SpreadsheetAction::Navigate(ArrowKey::Down))
        );
        assert_eq!(
            ck("ArrowUp"),
            Some(SpreadsheetAction::Navigate(ArrowKey::Up))
        );
    }

    #[wasm_bindgen_test]
    fn tab_navigates_right() {
        assert_eq!(
            classify_key("Tab", false, false, false, None),
            Some(SpreadsheetAction::Navigate(ArrowKey::Right))
        );
    }

    #[wasm_bindgen_test]
    fn shift_tab_navigates_left() {
        assert_eq!(
            classify_key("Tab", false, true, false, None),
            Some(SpreadsheetAction::Navigate(ArrowKey::Left))
        );
    }

    #[wasm_bindgen_test]
    fn enter_navigates_down() {
        assert_eq!(
            classify_key("Enter", false, false, false, None),
            Some(SpreadsheetAction::Navigate(ArrowKey::Down))
        );
    }

    #[wasm_bindgen_test]
    fn page_up_down() {
        assert_eq!(
            classify_key("PageDown", false, false, false, None),
            Some(SpreadsheetAction::PageDown)
        );
        assert_eq!(
            classify_key("PageUp", false, false, false, None),
            Some(SpreadsheetAction::PageUp)
        );
    }

    #[wasm_bindgen_test]
    fn home_end() {
        assert_eq!(
            classify_key("Home", false, false, false, None),
            Some(SpreadsheetAction::RowHome)
        );
        assert_eq!(
            classify_key("End", false, false, false, None),
            Some(SpreadsheetAction::RowEnd)
        );
    }

    #[wasm_bindgen_test]
    fn delete_and_escape() {
        assert_eq!(
            classify_key("Delete", false, false, false, None),
            Some(SpreadsheetAction::Delete)
        );
        assert_eq!(
            classify_key("Escape", false, false, false, None),
            Some(SpreadsheetAction::CancelEdit)
        );
    }

    #[wasm_bindgen_test]
    fn f2_enters_edit_mode() {
        assert_eq!(
            classify_key("F2", false, false, false, None),
            Some(SpreadsheetAction::EnterEditMode)
        );
    }

    #[wasm_bindgen_test]
    fn printable_chars_start_edit() {
        let start = |k: &str| Some(SpreadsheetAction::StartEdit(k.to_owned()));
        assert_eq!(classify_key("a", false, false, false, None), start("a"));
        assert_eq!(classify_key("=", false, false, false, None), start("="));
        assert_eq!(classify_key("1", false, false, false, None), start("1"));
        assert_eq!(classify_key(" ", false, false, false, None), start(" "));
    }

    #[wasm_bindgen_test]
    fn non_printable_returns_none() {
        let none = |k| classify_key(k, false, false, false, None);
        assert_eq!(none("F1"), None);
        assert_eq!(none("Shift"), None);
        assert_eq!(none("Control"), None);
        assert_eq!(none("Backspace"), None);
        assert_eq!(none("Alt"), None);
    }

    // ── classify_key: Ctrl combos ─────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn ctrl_z_y_undo_redo() {
        let c = |k| classify_key(k, true, false, false, None);
        assert_eq!(c("z"), Some(SpreadsheetAction::Undo));
        assert_eq!(c("Z"), Some(SpreadsheetAction::Undo)); // caps-lock resilience
        assert_eq!(c("y"), Some(SpreadsheetAction::Redo));
        assert_eq!(c("Y"), Some(SpreadsheetAction::Redo));
    }

    #[wasm_bindgen_test]
    fn ctrl_a_selects_all() {
        assert_eq!(
            classify_key("a", true, false, false, None),
            Some(SpreadsheetAction::SelectAll)
        );
        assert_eq!(
            classify_key("A", true, false, false, None),
            Some(SpreadsheetAction::SelectAll)
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_c_x_v_clipboard() {
        let c = |k| classify_key(k, true, false, false, None);
        assert_eq!(c("c"), Some(SpreadsheetAction::Copy));
        assert_eq!(c("C"), Some(SpreadsheetAction::Copy));
        assert_eq!(c("x"), Some(SpreadsheetAction::Cut));
        assert_eq!(c("X"), Some(SpreadsheetAction::Cut));
        assert_eq!(c("v"), Some(SpreadsheetAction::Paste));
        assert_eq!(c("V"), Some(SpreadsheetAction::Paste));
    }

    #[wasm_bindgen_test]
    fn ctrl_home_end_jump() {
        assert_eq!(
            classify_key("Home", true, false, false, None),
            Some(SpreadsheetAction::JumpToA1)
        );
        assert_eq!(
            classify_key("End", true, false, false, None),
            Some(SpreadsheetAction::JumpToLastCell)
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_arrows_navigate_to_edge() {
        let c = |k| classify_key(k, true, false, false, None);
        assert_eq!(
            c("ArrowRight"),
            Some(SpreadsheetAction::NavigateEdge(ArrowKey::Right))
        );
        assert_eq!(
            c("ArrowLeft"),
            Some(SpreadsheetAction::NavigateEdge(ArrowKey::Left))
        );
        assert_eq!(
            c("ArrowUp"),
            Some(SpreadsheetAction::NavigateEdge(ArrowKey::Up))
        );
        assert_eq!(
            c("ArrowDown"),
            Some(SpreadsheetAction::NavigateEdge(ArrowKey::Down))
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_minus_deletes_rows() {
        assert_eq!(
            classify_key("-", true, false, false, None),
            Some(SpreadsheetAction::DeleteRows)
        );
    }

    // ── classify_key: Ctrl+Shift combos ──────────────────────────────────────

    #[wasm_bindgen_test]
    fn ctrl_shift_delete_clears_all() {
        assert_eq!(
            classify_key("Delete", true, true, false, None),
            Some(SpreadsheetAction::ClearAll)
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_plus_inserts_rows() {
        assert_eq!(
            classify_key("=", true, true, false, None),
            Some(SpreadsheetAction::InsertRows)
        );
        assert_eq!(
            classify_key("+", true, true, false, None),
            Some(SpreadsheetAction::InsertRows)
        );
    }

    // ── classify_key: Ctrl+Alt and Ctrl+Shift+Alt ─────────────────────────────

    #[wasm_bindgen_test]
    fn ctrl_alt_minus_deletes_columns() {
        assert_eq!(
            classify_key("-", true, false, true, None),
            Some(SpreadsheetAction::DeleteColumns)
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_alt_plus_inserts_columns() {
        assert_eq!(
            classify_key("=", true, true, true, None),
            Some(SpreadsheetAction::InsertColumns)
        );
        assert_eq!(
            classify_key("+", true, true, true, None),
            Some(SpreadsheetAction::InsertColumns)
        );
    }

    // ── classify_key: Alt-only ────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn alt_arrows_switch_sheet() {
        assert_eq!(
            classify_key("ArrowDown", false, false, true, None),
            Some(SpreadsheetAction::SwitchSheet(1))
        );
        assert_eq!(
            classify_key("ArrowUp", false, false, true, None),
            Some(SpreadsheetAction::SwitchSheet(-1))
        );
    }

    // ── classify_key: Shift-only ──────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn shift_arrows_expand_selection() {
        let s = |k| classify_key(k, false, true, false, None);
        assert_eq!(
            s("ArrowRight"),
            Some(SpreadsheetAction::ExpandSelection(ArrowKey::Right))
        );
        assert_eq!(
            s("ArrowLeft"),
            Some(SpreadsheetAction::ExpandSelection(ArrowKey::Left))
        );
        assert_eq!(
            s("ArrowUp"),
            Some(SpreadsheetAction::ExpandSelection(ArrowKey::Up))
        );
        assert_eq!(
            s("ArrowDown"),
            Some(SpreadsheetAction::ExpandSelection(ArrowKey::Down))
        );
    }

    // ── classify_key: while editing (Accept mode) ─────────────────────────────

    #[wasm_bindgen_test]
    fn accept_mode_enter_tab_commit() {
        let e = accept_cell();
        assert_eq!(
            classify_key("Enter", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Down))
        );
        assert_eq!(
            classify_key("Tab", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Right))
        );
        assert_eq!(
            classify_key("Tab", false, true, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Left))
        );
    }

    #[wasm_bindgen_test]
    fn accept_mode_escape_cancels() {
        let e = accept_cell();
        assert_eq!(
            classify_key("Escape", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CancelEdit)
        );
    }

    #[wasm_bindgen_test]
    fn accept_mode_arrows_commit_and_navigate() {
        let e = accept_cell();
        assert_eq!(
            classify_key("ArrowDown", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Down))
        );
        assert_eq!(
            classify_key("ArrowUp", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Up))
        );
        assert_eq!(
            classify_key("ArrowLeft", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Left))
        );
        assert_eq!(
            classify_key("ArrowRight", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Right))
        );
    }

    // ── classify_key: while editing (Edit mode) ───────────────────────────────

    #[wasm_bindgen_test]
    fn edit_mode_arrows_are_unhandled() {
        // In Edit mode arrows move the text cursor — not a SpreadsheetAction.
        let e = edit_cell();
        assert_eq!(
            classify_key("ArrowDown", false, false, false, Some(&e)),
            None
        );
        assert_eq!(classify_key("ArrowUp", false, false, false, Some(&e)), None);
        assert_eq!(
            classify_key("ArrowLeft", false, false, false, Some(&e)),
            None
        );
        assert_eq!(
            classify_key("ArrowRight", false, false, false, Some(&e)),
            None
        );
    }

    #[wasm_bindgen_test]
    fn edit_mode_enter_and_escape_still_work() {
        let e = edit_cell();
        assert_eq!(
            classify_key("Enter", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CommitAndNavigate(ArrowKey::Down))
        );
        assert_eq!(
            classify_key("Escape", false, false, false, Some(&e)),
            Some(SpreadsheetAction::CancelEdit)
        );
    }

    // ── classify_key: editing mode ignores Ctrl shortcuts ─────────────────────

    #[wasm_bindgen_test]
    fn editing_mode_ctrl_c_returns_none() {
        // While editing, Ctrl+C/V/Z are handled by the textarea natively.
        let e = edit_cell();
        assert_eq!(classify_key("c", true, false, false, Some(&e)), None);
        assert_eq!(classify_key("v", true, false, false, Some(&e)), None);
        assert_eq!(classify_key("z", true, false, false, Some(&e)), None);
        assert_eq!(classify_key("a", true, false, false, Some(&e)), None);
    }

    // ── selection_bounds ──────────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn selection_bounds_normal_order() {
        let ((r_min, r_max), (c_min, c_max)) = selection_bounds([1, 2, 5, 6]);
        assert_eq!((r_min, r_max, c_min, c_max), (1, 5, 2, 6));
    }

    #[wasm_bindgen_test]
    fn selection_bounds_reversed_normalizes() {
        let ((r_min, r_max), (c_min, c_max)) = selection_bounds([5, 6, 1, 2]);
        assert_eq!((r_min, r_max, c_min, c_max), (1, 5, 2, 6));
    }

    #[wasm_bindgen_test]
    fn selection_bounds_single_cell() {
        let ((r_min, r_max), (c_min, c_max)) = selection_bounds([3, 4, 3, 4]);
        assert_eq!((r_min, r_max, c_min, c_max), (3, 3, 4, 4));
    }

    // ── execute: navigation ───────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn execute_navigate_down_advances_row() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::Navigate(ArrowKey::Down), model, &state);
            let row = model.with_value(|m| m.get_selected_view().row);
            assert_eq!(row, 2);
        });
    }

    #[wasm_bindgen_test]
    fn execute_navigate_right_advances_column() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::Navigate(ArrowKey::Right), model, &state);
            let col = model.with_value(|m| m.get_selected_view().column);
            assert_eq!(col, 2);
        });
    }

    #[wasm_bindgen_test]
    fn execute_jump_to_a1_resets_position() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            // move away from A1 first
            execute(&SpreadsheetAction::Navigate(ArrowKey::Down), model, &state);
            execute(&SpreadsheetAction::Navigate(ArrowKey::Right), model, &state);
            execute(&SpreadsheetAction::JumpToA1, model, &state);
            let v = model.with_value(|m| m.get_selected_view());
            assert_eq!((v.row, v.column), (1, 1));
        });
    }

    // ── execute: editing ──────────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn execute_start_edit_sets_editing_cell() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::StartEdit("=SUM".to_owned()),
                model,
                &state,
            );
            let cell = state.editing_cell.get_untracked();
            assert!(cell.is_some());
            assert_eq!(cell.unwrap().text, "=SUM");
        });
    }

    #[wasm_bindgen_test]
    fn execute_cancel_edit_clears_editing_state() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::StartEdit("hello".to_owned()),
                model,
                &state,
            );
            assert!(state.editing_cell.get_untracked().is_some());
            execute(&SpreadsheetAction::CancelEdit, model, &state);
            assert!(state.editing_cell.get_untracked().is_none());
            assert!(state.point_range.get_untracked().is_none());
        });
    }

    #[wasm_bindgen_test]
    fn execute_commit_writes_value_and_navigates() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::StartEdit("42".to_owned()),
                model,
                &state,
            );
            execute(
                &SpreadsheetAction::CommitAndNavigate(ArrowKey::Down),
                model,
                &state,
            );
            // value committed to A1
            let val = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(val, "42");
            // editing state cleared
            assert!(state.editing_cell.get_untracked().is_none());
            // cursor moved to row 2
            let row = model.with_value(|m| m.get_selected_view().row);
            assert_eq!(row, 2);
        });
    }

    #[wasm_bindgen_test]
    fn execute_enter_edit_mode_loads_existing_content() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            // Write "hello" directly to the model so it's already there before F2.
            model.update_value(|m| {
                m.set_user_input(1, 1, 1, "hello").ok();
                m.evaluate();
            });
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::EnterEditMode, model, &state);
            let cell = state.editing_cell.get_untracked().unwrap();
            assert_eq!(cell.mode, EditMode::Edit);
            assert_eq!(cell.text, "hello");
        });
    }

    // ── execute: mutations ────────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn execute_delete_clears_cell_content() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            model.update_value(|m| {
                m.set_user_input(1, 1, 1, "data").ok();
                m.evaluate();
            });
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::Delete, model, &state);
            let val = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(val, "");
        });
    }

    #[wasm_bindgen_test]
    fn execute_undo_redo_roundtrip() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            model.update_value(|m| {
                m.set_user_input(1, 1, 1, "42").ok();
                m.evaluate();
            });
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::Undo, model, &state);
            let after_undo =
                model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(after_undo, "");
            execute(&SpreadsheetAction::Redo, model, &state);
            let after_redo =
                model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(after_redo, "42");
        });
    }

    #[wasm_bindgen_test]
    fn execute_insert_row_pushes_content_down() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            // Write "original" to A1 (row 1, col 1).
            model.update_value(|m| {
                m.set_user_input(1, 1, 1, "original").ok();
                m.evaluate();
            });
            let state = crate::state::WorkbookState::new();
            // Cursor at row 1 → InsertRows inserts above, pushing "original" to A2.
            execute(&SpreadsheetAction::InsertRows, model, &state);
            let a1 = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            let a2 = model.with_value(|m| m.get_formatted_cell_value(1, 2, 1).unwrap_or_default());
            assert_eq!(a1, "");
            assert_eq!(a2, "original");
        });
    }

    #[wasm_bindgen_test]
    fn execute_delete_row_pulls_content_up() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            // Put content in A2 (row 2), then delete row 1 to bring it to A1.
            model.update_value(|m| {
                m.set_user_input(1, 2, 1, "data").ok();
                m.evaluate();
            });
            let state = crate::state::WorkbookState::new();
            // Cursor at row 1 → DeleteRows removes row 1, A2 becomes A1.
            execute(&SpreadsheetAction::DeleteRows, model, &state);
            let a1 = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(a1, "data");
        });
    }
}
