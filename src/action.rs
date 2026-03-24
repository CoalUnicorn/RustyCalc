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
//      Call `mutate(model, state, needs_evaluate, |m| { … })` for model
//      mutations.  Pass `evaluate: true` whenever formula results may change
//      (cell writes, row/column inserts/deletes, undo/redo of those).
//      Example:
//
//          SpreadsheetAction::DuplicateRow => {
//              mutate(model, state, true, |m| {
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
// ── IRONCALC UserModel API REFERENCE ──────────────────────────────────────────
//
// Everything here lives in `ironcalc_base::UserModel<'static>`.
// Use `mutate(model, state, evaluate, |m| { … })` to call these inside execute.
//
// CELL READING / WRITING
//   m.set_user_input(sheet, row, col, value)   → write a formula or literal
//   m.get_cell_content(sheet, row, col)         → raw formula / literal string
//   m.get_formatted_cell_value(sheet, row, col) → display string (e.g. "1,234")
//   m.get_cell_type(sheet, row, col)            → CellType enum
//
// EVALUATION
//   m.evaluate()                                → recalculate all dirty cells
//   m.pause_evaluation() / m.resume_evaluation()
//
// UNDO / REDO
//   m.undo()  m.redo()  m.can_undo()  m.can_redo()
//
// SELECTION & NAVIGATION  (these also update the internal viewport/scroll)
//   m.get_selected_view()                       → SelectedView { sheet, row, column, range, … }
//   m.set_selected_cell(row, col)
//   m.set_selected_range(r1, c1, r2, c2)
//   m.on_expand_selected_range(key)             → key = "ArrowRight" / "ArrowLeft" / …
//   m.on_arrow_right/left/up/down()
//   m.on_navigate_to_edge_in_direction(NavigationDirection::…)
//   m.on_page_down() / m.on_page_up()
//   m.on_area_selecting(target_row, target_col) → drag-selection
//   m.set_top_left_visible_cell(top_row, left_col)
//   m.get_scroll_x() / m.get_scroll_y()
//   m.set_window_width(w) / m.set_window_height(h)
//
// SHEET MANAGEMENT
//   m.get_worksheets_properties()               → Vec<SheetProperties> (id, name, state, color, …)
//   m.get_selected_sheet()
//   m.set_selected_sheet(sheet_id)
//   m.new_sheet()
//   m.delete_sheet(sheet_id)
//   m.rename_sheet(sheet_id, new_name)
//   m.hide_sheet(sheet_id) / m.unhide_sheet(sheet_id)
//   m.set_sheet_color(sheet_id, "#RRGGBB")
//
// ROW / COLUMN OPERATIONS
//   m.insert_rows(sheet, row, count)
//   m.delete_rows(sheet, row, count)
//   m.insert_columns(sheet, col, count)
//   m.delete_columns(sheet, col, count)
//   m.move_rows_action(sheet, row, count, delta)
//   m.move_columns_action(sheet, col, count, delta)
//   m.set_rows_height(sheet, row_start, row_end, height_px)
//   m.set_columns_width(sheet, col_start, col_end, width_px)
//   m.get_row_height(sheet, row)
//   m.get_column_width(sheet, col)
//   m.set_rows_hidden(sheet, row_start, row_end, hidden)
//   m.set_columns_hidden(sheet, col_start, col_end, hidden)
//
// RANGE CLEARING
//   m.range_clear_contents(&area)              → values only, keep formatting
//   m.range_clear_formatting(&area)            → formatting only
//   m.range_clear_all(&area)                   → both
//   // Build Area with make_area(sheet, r1, c1, r2, c2) from this file.
//
// CLIPBOARD
//   m.copy_to_clipboard()                      → Clipboard
//   m.paste_from_clipboard(src_sheet, src_range, &clipboard_data, is_cut)
//   m.paste_csv_string(&area, csv_str)
//
// STYLING
//   m.get_cell_style(sheet, row, col)           → Style
//   m.update_range_style(&area, style_path, value)
//   m.on_paste_styles(&styles)
//   m.set_area_with_border(&area, &border_area)
//   m.set_show_grid_lines(sheet, bool)
//
// AUTO-FILL
//   m.auto_fill_rows(&source_area, to_row)
//   m.auto_fill_columns(&source_area, to_col)
//
// FROZEN PANES
//   m.set_frozen_rows_count(sheet, n)
//   m.set_frozen_columns_count(sheet, n)
//   m.get_frozen_rows_count(sheet) / m.get_frozen_columns_count(sheet)
//
// NAMED RANGES
//   m.get_defined_name_list()                  → Vec<(name, scope, formula)>
//   m.new_defined_name(name, scope, formula)
//   m.update_defined_name(name, scope, new_name, new_scope, new_formula)
//   m.delete_defined_name(name, scope)
//   m.is_valid_defined_name(name, scope, formula)
//
// LOCALE / TIMEZONE / LANGUAGE
//   m.set_locale(locale_id)  m.get_locale()
//   m.set_timezone(tz)       m.get_timezone()
//   m.set_language(lang_id)  m.get_language()
//   m.get_fmt_settings()     → FmtSettings (decimal/thousands separators, …)
//
// SERIALISATION
//   m.to_bytes() / UserModel::from_bytes(bytes, lang_id)
//
// ==============================================================================

use ironcalc_base::expressions::types::Area;
use ironcalc_base::worksheet::NavigationDirection;
use ironcalc_base::UserModel;
use leptos::prelude::*;

use crate::state::{EditFocus, EditMode, EditingCell, ModelStore, WorkbookState};
use crate::storage;

// ── Direction ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn to_nav(self) -> NavigationDirection {
        match self {
            Self::Up => NavigationDirection::Up,
            Self::Down => NavigationDirection::Down,
            Self::Left => NavigationDirection::Left,
            Self::Right => NavigationDirection::Right,
        }
    }

    /// Key string expected by `UserModel::on_expand_selected_range`.
    fn to_arrow_key(self) -> &'static str {
        match self {
            Self::Up => "ArrowUp",
            Self::Down => "ArrowDown",
            Self::Left => "ArrowLeft",
            Self::Right => "ArrowRight",
        }
    }

    fn navigate(self, m: &mut UserModel<'static>) -> Result<(), String> {
        match self {
            Self::Up => m.on_arrow_up(),
            Self::Down => m.on_arrow_down(),
            Self::Left => m.on_arrow_left(),
            Self::Right => m.on_arrow_right(),
        }
    }
}

// ── SpreadsheetAction ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SpreadsheetAction {
    // ── Navigation ──────────────────────────────────────────────────────────
    /// Move the active cell one step in a direction.
    Navigate(Direction),
    /// Ctrl+Arrow: jump to the data boundary in a direction.
    NavigateEdge(Direction),
    /// Ctrl+Home: jump to A1.
    JumpToA1,
    /// Ctrl+End: jump to the last used cell.
    JumpToLastCell,
    /// Shift+Arrow: extend the selection range.
    ExpandSelection(Direction),
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
    CommitAndNavigate(Direction),
    /// Escape: discard the edit buffer without writing to the model.
    CancelEdit,

    // ── Structural mutations ─────────────────────────────────────────────────
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
    use Direction::*;
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
        // Z/Y are lowercased to handle caps-lock correctly.
        match key.to_lowercase().as_str() {
            "z" => return Some(Undo),
            "y" => return Some(Redo),
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
pub fn execute(action: &SpreadsheetAction, model: ModelStore, state: &WorkbookState) {
    match action {
        SpreadsheetAction::Navigate(dir) => {
            mutate(model, state, false, |m| { dir.navigate(m).ok(); });
        }
        SpreadsheetAction::NavigateEdge(dir) => {
            mutate(model, state, false, |m| {
                m.on_navigate_to_edge_in_direction(dir.to_nav()).ok();
            });
        }
        SpreadsheetAction::JumpToA1 => {
            mutate(model, state, false, |m| { m.set_selected_cell(1, 1).ok(); });
        }
        SpreadsheetAction::JumpToLastCell => {
            mutate(model, state, false, |m| {
                m.on_navigate_to_edge_in_direction(NavigationDirection::Down).ok();
                m.on_navigate_to_edge_in_direction(NavigationDirection::Right).ok();
            });
        }
        SpreadsheetAction::ExpandSelection(dir) => {
            mutate(model, state, false, |m| {
                m.on_expand_selected_range(dir.to_arrow_key()).ok();
            });
        }
        SpreadsheetAction::PageDown => {
            mutate(model, state, false, |m| { m.on_page_down().ok(); });
        }
        SpreadsheetAction::PageUp => {
            mutate(model, state, false, |m| { m.on_page_up().ok(); });
        }
        SpreadsheetAction::RowHome => {
            mutate(model, state, false, |m| {
                let row = m.get_selected_view().row;
                m.set_selected_cell(row, 1).ok();
            });
        }
        SpreadsheetAction::RowEnd => {
            mutate(model, state, false, |m| {
                m.on_navigate_to_edge_in_direction(NavigationDirection::Right).ok();
            });
        }
        SpreadsheetAction::SwitchSheet(delta) => {
            let delta = *delta;
            mutate(model, state, false, move |m| {
                let current = m.get_selected_view().sheet;
                let visible: Vec<u32> = m
                    .get_worksheets_properties()
                    .iter()
                    .filter(|s| s.state == "visible")
                    .map(|s| s.sheet_id)
                    .collect();
                if let Some(pos) = visible.iter().position(|&id| id == current) {
                    let next =
                        (pos as i32 + delta).rem_euclid(visible.len() as i32) as usize;
                    m.set_selected_sheet(visible[next]).ok();
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
                    m.set_user_input(edit.sheet, edit.row, edit.col, &edit.text)
                        .ok();
                    m.evaluate();
                });
                // Clear all edit-related state.
                state.editing_cell.set(None);
                state.point_range.set(None);
                state.point_ref_span.set(None);
                // Persist the committed change immediately.
                let uuid = state.current_uuid.get_untracked();
                if !uuid.is_empty() {
                    model.with_value(|m| storage::save(&uuid, m));
                }
                // Navigate to the next cell and redraw.
                model.update_value(|m| { dir.navigate(m).ok(); });
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
        SpreadsheetAction::Delete => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                m.range_clear_contents(&make_area(v.sheet, r1, c1, r2, c2))
                    .ok();
            });
        }
        SpreadsheetAction::ClearAll => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let [r1, c1, r2, c2] = v.range;
                m.range_clear_all(&make_area(v.sheet, r1, c1, r2, c2)).ok();
            });
        }
        SpreadsheetAction::Undo => {
            mutate(model, state, false, |m| { m.undo().ok(); });
        }
        SpreadsheetAction::Redo => {
            mutate(model, state, false, |m| { m.redo().ok(); });
        }
        SpreadsheetAction::InsertRows => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                m.insert_rows(v.sheet, r_min, r_max - r_min + 1).ok();
            });
        }
        SpreadsheetAction::InsertColumns => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                m.insert_columns(v.sheet, c_min, c_max - c_min + 1).ok();
            });
        }
        SpreadsheetAction::DeleteRows => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let ((r_min, r_max), _) = selection_bounds(v.range);
                m.delete_rows(v.sheet, r_min, r_max - r_min + 1).ok();
            });
        }
        SpreadsheetAction::DeleteColumns => {
            mutate(model, state, true, |m| {
                let v = m.get_selected_view();
                let (_, (c_min, c_max)) = selection_bounds(v.range);
                m.delete_columns(v.sheet, c_min, c_max - c_min + 1).ok();
            });
        }
    }
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Run `f` on the model, optionally call `evaluate`, then trigger a redraw.
fn mutate(
    model: ModelStore,
    state: &WorkbookState,
    evaluate: bool,
    f: impl FnOnce(&mut UserModel<'static>),
) {
    model.update_value(|m| {
        f(m);
        if evaluate {
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
        assert_eq!(ck("ArrowRight"), Some(SpreadsheetAction::Navigate(Direction::Right)));
        assert_eq!(ck("ArrowLeft"),  Some(SpreadsheetAction::Navigate(Direction::Left)));
        assert_eq!(ck("ArrowDown"),  Some(SpreadsheetAction::Navigate(Direction::Down)));
        assert_eq!(ck("ArrowUp"),    Some(SpreadsheetAction::Navigate(Direction::Up)));
    }

    #[wasm_bindgen_test]
    fn tab_navigates_right() {
        assert_eq!(
            classify_key("Tab", false, false, false, None),
            Some(SpreadsheetAction::Navigate(Direction::Right))
        );
    }

    #[wasm_bindgen_test]
    fn shift_tab_navigates_left() {
        assert_eq!(
            classify_key("Tab", false, true, false, None),
            Some(SpreadsheetAction::Navigate(Direction::Left))
        );
    }

    #[wasm_bindgen_test]
    fn enter_navigates_down() {
        assert_eq!(
            classify_key("Enter", false, false, false, None),
            Some(SpreadsheetAction::Navigate(Direction::Down))
        );
    }

    #[wasm_bindgen_test]
    fn page_up_down() {
        assert_eq!(classify_key("PageDown", false, false, false, None), Some(SpreadsheetAction::PageDown));
        assert_eq!(classify_key("PageUp",   false, false, false, None), Some(SpreadsheetAction::PageUp));
    }

    #[wasm_bindgen_test]
    fn home_end() {
        assert_eq!(classify_key("Home", false, false, false, None), Some(SpreadsheetAction::RowHome));
        assert_eq!(classify_key("End",  false, false, false, None), Some(SpreadsheetAction::RowEnd));
    }

    #[wasm_bindgen_test]
    fn delete_and_escape() {
        assert_eq!(classify_key("Delete", false, false, false, None), Some(SpreadsheetAction::Delete));
        assert_eq!(classify_key("Escape", false, false, false, None), Some(SpreadsheetAction::CancelEdit));
    }

    #[wasm_bindgen_test]
    fn f2_enters_edit_mode() {
        assert_eq!(classify_key("F2", false, false, false, None), Some(SpreadsheetAction::EnterEditMode));
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
        assert_eq!(none("F1"),       None);
        assert_eq!(none("Shift"),    None);
        assert_eq!(none("Control"),  None);
        assert_eq!(none("Backspace"), None);
        assert_eq!(none("Alt"),      None);
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
    fn ctrl_home_end_jump() {
        assert_eq!(classify_key("Home", true, false, false, None), Some(SpreadsheetAction::JumpToA1));
        assert_eq!(classify_key("End",  true, false, false, None), Some(SpreadsheetAction::JumpToLastCell));
    }

    #[wasm_bindgen_test]
    fn ctrl_arrows_navigate_to_edge() {
        let c = |k| classify_key(k, true, false, false, None);
        assert_eq!(c("ArrowRight"), Some(SpreadsheetAction::NavigateEdge(Direction::Right)));
        assert_eq!(c("ArrowLeft"),  Some(SpreadsheetAction::NavigateEdge(Direction::Left)));
        assert_eq!(c("ArrowUp"),    Some(SpreadsheetAction::NavigateEdge(Direction::Up)));
        assert_eq!(c("ArrowDown"),  Some(SpreadsheetAction::NavigateEdge(Direction::Down)));
    }

    #[wasm_bindgen_test]
    fn ctrl_minus_deletes_rows() {
        assert_eq!(classify_key("-", true, false, false, None), Some(SpreadsheetAction::DeleteRows));
    }

    // ── classify_key: Ctrl+Shift combos ──────────────────────────────────────

    #[wasm_bindgen_test]
    fn ctrl_shift_delete_clears_all() {
        assert_eq!(classify_key("Delete", true, true, false, None), Some(SpreadsheetAction::ClearAll));
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_plus_inserts_rows() {
        assert_eq!(classify_key("=", true, true, false, None), Some(SpreadsheetAction::InsertRows));
        assert_eq!(classify_key("+", true, true, false, None), Some(SpreadsheetAction::InsertRows));
    }

    // ── classify_key: Ctrl+Alt and Ctrl+Shift+Alt ─────────────────────────────

    #[wasm_bindgen_test]
    fn ctrl_alt_minus_deletes_columns() {
        assert_eq!(classify_key("-", true, false, true, None), Some(SpreadsheetAction::DeleteColumns));
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_alt_plus_inserts_columns() {
        assert_eq!(classify_key("=", true, true, true, None), Some(SpreadsheetAction::InsertColumns));
        assert_eq!(classify_key("+", true, true, true, None), Some(SpreadsheetAction::InsertColumns));
    }

    // ── classify_key: Alt-only ────────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn alt_arrows_switch_sheet() {
        assert_eq!(classify_key("ArrowDown", false, false, true, None), Some(SpreadsheetAction::SwitchSheet(1)));
        assert_eq!(classify_key("ArrowUp",   false, false, true, None), Some(SpreadsheetAction::SwitchSheet(-1)));
    }

    // ── classify_key: Shift-only ──────────────────────────────────────────────

    #[wasm_bindgen_test]
    fn shift_arrows_expand_selection() {
        let s = |k| classify_key(k, false, true, false, None);
        assert_eq!(s("ArrowRight"), Some(SpreadsheetAction::ExpandSelection(Direction::Right)));
        assert_eq!(s("ArrowLeft"),  Some(SpreadsheetAction::ExpandSelection(Direction::Left)));
        assert_eq!(s("ArrowUp"),    Some(SpreadsheetAction::ExpandSelection(Direction::Up)));
        assert_eq!(s("ArrowDown"),  Some(SpreadsheetAction::ExpandSelection(Direction::Down)));
    }

    // ── classify_key: while editing (Accept mode) ─────────────────────────────

    #[wasm_bindgen_test]
    fn accept_mode_enter_tab_commit() {
        let e = accept_cell();
        assert_eq!(classify_key("Enter", false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Down)));
        assert_eq!(classify_key("Tab",   false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Right)));
        assert_eq!(classify_key("Tab",   false, true,  false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Left)));
    }

    #[wasm_bindgen_test]
    fn accept_mode_escape_cancels() {
        let e = accept_cell();
        assert_eq!(classify_key("Escape", false, false, false, Some(&e)), Some(SpreadsheetAction::CancelEdit));
    }

    #[wasm_bindgen_test]
    fn accept_mode_arrows_commit_and_navigate() {
        let e = accept_cell();
        assert_eq!(classify_key("ArrowDown",  false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Down)));
        assert_eq!(classify_key("ArrowUp",    false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Up)));
        assert_eq!(classify_key("ArrowLeft",  false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Left)));
        assert_eq!(classify_key("ArrowRight", false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Right)));
    }

    // ── classify_key: while editing (Edit mode) ───────────────────────────────

    #[wasm_bindgen_test]
    fn edit_mode_arrows_are_unhandled() {
        // In Edit mode arrows move the text cursor — not a SpreadsheetAction.
        let e = edit_cell();
        assert_eq!(classify_key("ArrowDown",  false, false, false, Some(&e)), None);
        assert_eq!(classify_key("ArrowUp",    false, false, false, Some(&e)), None);
        assert_eq!(classify_key("ArrowLeft",  false, false, false, Some(&e)), None);
        assert_eq!(classify_key("ArrowRight", false, false, false, Some(&e)), None);
    }

    #[wasm_bindgen_test]
    fn edit_mode_enter_and_escape_still_work() {
        let e = edit_cell();
        assert_eq!(classify_key("Enter",  false, false, false, Some(&e)), Some(SpreadsheetAction::CommitAndNavigate(Direction::Down)));
        assert_eq!(classify_key("Escape", false, false, false, Some(&e)), Some(SpreadsheetAction::CancelEdit));
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
            execute(&SpreadsheetAction::Navigate(Direction::Down), model, &state);
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
            execute(&SpreadsheetAction::Navigate(Direction::Right), model, &state);
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
            execute(&SpreadsheetAction::Navigate(Direction::Down), model, &state);
            execute(&SpreadsheetAction::Navigate(Direction::Right), model, &state);
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
            execute(&SpreadsheetAction::StartEdit("=SUM".to_owned()), model, &state);
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
            execute(&SpreadsheetAction::StartEdit("hello".to_owned()), model, &state);
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
            execute(&SpreadsheetAction::StartEdit("42".to_owned()), model, &state);
            execute(&SpreadsheetAction::CommitAndNavigate(Direction::Down), model, &state);
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
            model.update_value(|m| { m.set_user_input(1, 1, 1, "hello").ok(); m.evaluate(); });
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
            model.update_value(|m| { m.set_user_input(1, 1, 1, "data").ok(); m.evaluate(); });
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
            model.update_value(|m| { m.set_user_input(1, 1, 1, "42").ok(); m.evaluate(); });
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::Undo, model, &state);
            let after_undo = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(after_undo, "");
            execute(&SpreadsheetAction::Redo, model, &state);
            let after_redo = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
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
            model.update_value(|m| { m.set_user_input(1, 1, 1, "original").ok(); m.evaluate(); });
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
            model.update_value(|m| { m.set_user_input(1, 2, 1, "data").ok(); m.evaluate(); });
            let state = crate::state::WorkbookState::new();
            // Cursor at row 1 → DeleteRows removes row 1, A2 becomes A1.
            execute(&SpreadsheetAction::DeleteRows, model, &state);
            let a1 = model.with_value(|m| m.get_formatted_cell_value(1, 1, 1).unwrap_or_default());
            assert_eq!(a1, "data");
        });
    }
}
