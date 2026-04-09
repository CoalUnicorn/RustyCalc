// Key -> SpreadsheetAction -> model mutation pipeline.
//
// SpreadsheetAction is a thin wrapper around category-specific sub-enums.
// Each category lives in its own module with its own execute function:
//   nav.rs       - arrow keys, page up/down, home/end, sheet switching
//   edit.rs      - start/commit/cancel cell editing
//   format.rs    - bold, italic, underline, strikethrough, font size/family
//   structure.rs - delete, clear, undo/redo, insert/delete rows/columns
//
// See docs/adding-actions.md for how to add or modify actions.

use leptos::prelude::WithValue;

use crate::input::{
    edit::{execute_edit, EditAction},
    format::{execute_format, FormatAction},
    nav::{execute_nav, NavAction},
    structure::{execute_struct, StructAction},
};
use crate::model::{style_types::HexColor, ArrowKey, SafeFontFamily};
use crate::state::EditMode;
use crate::state::{EditingCell, ModelStore, WorkbookState};
use crate::storage;

// SpreadsheetAction

/// Top-level action dispatched from a keyboard event.
///
/// [`classify_key`] maps a key + modifier combination to one of these variants.
/// [`execute`] dispatches to the appropriate category module (`nav`, `edit`,
/// `format`, `structure`). `Copy`, `Cut`, and `Paste` are handled inline in
/// `Workbook` because they need `AppClipboard` and async OS clipboard APIs.
#[derive(Debug, Clone, PartialEq)]
pub enum SpreadsheetAction {
    Nav(NavAction),
    Edit(EditAction),
    Format(FormatAction),
    Structure(StructAction),
    /// Clipboard actions are handled by the Workbook component directly
    /// (they need the AppClipboard store and async OS clipboard APIs).
    Copy,
    Cut,
    Paste,
}

// Key classification

/// Keyboard modifier state at the time of a key event.
///
/// Replaces three positional `bool` parameters in `classify_key` - callers
/// can no longer silently swap `ctrl` and `alt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyMod {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Map a keyboard event to a `SpreadsheetAction`, or `None` if unhandled.
///
/// This function is pure - no side effects, no DOM access.
///
/// **Point-mode arrow navigation** is excluded: it requires reading the
/// textarea cursor position from the DOM, so it is handled as a pre-check
/// in the keydown closure before this function is called.
pub fn classify_key(
    key: &str,
    mods: KeyMod,
    edit: Option<&EditingCell>,
) -> Option<SpreadsheetAction> {
    let ctrl = mods.ctrl;
    let shift = mods.shift;
    let alt = mods.alt;
    use ArrowKey::*;
    use SpreadsheetAction::*;

    // While editing
    if let Some(e) = edit {
        return match key {
            "Enter" => Some(Edit(EditAction::CommitAndNavigate(Down))),
            "Tab" if shift => Some(Edit(EditAction::CommitAndNavigate(Left))),
            "Tab" => Some(Edit(EditAction::CommitAndNavigate(Right))),
            "Escape" => Some(Edit(EditAction::Cancel)),
            "ArrowDown" if e.mode == EditMode::Accept => {
                Some(Edit(EditAction::CommitAndNavigate(Down)))
            }
            "ArrowUp" if e.mode == EditMode::Accept => {
                Some(Edit(EditAction::CommitAndNavigate(Up)))
            }
            "ArrowLeft" if e.mode == EditMode::Accept => {
                Some(Edit(EditAction::CommitAndNavigate(Left)))
            }
            "ArrowRight" if e.mode == EditMode::Accept => {
                Some(Edit(EditAction::CommitAndNavigate(Right)))
            }
            _ => None,
        };
    }

    // Not editing

    // Ctrl-only (no shift, no alt).
    if ctrl && !shift && !alt {
        match key.to_lowercase().as_str() {
            "z" => return Some(Structure(StructAction::Undo)),
            "y" => return Some(Structure(StructAction::Redo)),
            "a" => return Some(Nav(NavAction::SelectAll)),
            "b" => return Some(Format(FormatAction::ToggleBold)),
            "i" => return Some(Format(FormatAction::ToggleItalic)),
            "u" => return Some(Format(FormatAction::ToggleUnderline)),
            "c" => return Some(Copy),
            "x" => return Some(Cut),
            "v" => return Some(Paste),
            _ => {}
        }
        return match key {
            "Home" => Some(Nav(NavAction::JumpToA1)),
            "End" => Some(Nav(NavAction::JumpToLastCell)),
            "ArrowRight" => Some(Nav(NavAction::Edge(Right))),
            "ArrowLeft" => Some(Nav(NavAction::Edge(Left))),
            "ArrowUp" => Some(Nav(NavAction::Edge(Up))),
            "ArrowDown" => Some(Nav(NavAction::Edge(Down))),
            "-" => Some(Structure(StructAction::DeleteRows)),
            _ => None,
        };
    }

    // Ctrl+Alt (no shift): delete columns.
    if ctrl && !shift && alt {
        return match key {
            "-" => Some(Structure(StructAction::DeleteColumns)),
            _ => None,
        };
    }

    // Ctrl+Shift (no alt): structural edits.
    if ctrl && shift && !alt {
        return match key {
            "Delete" => Some(Structure(StructAction::ClearAll)),
            "=" | "+" => Some(Structure(StructAction::InsertRows)),
            _ => None,
        };
    }

    // Ctrl+Shift+Alt: insert columns.
    if ctrl && shift && alt {
        return match key {
            "=" | "+" => Some(Structure(StructAction::InsertColumns)),
            _ => None,
        };
    }

    // Alt-only (no ctrl, no shift): sheet navigation.
    if alt && !ctrl && !shift {
        return match key {
            "ArrowDown" => Some(Nav(NavAction::SwitchSheet(1))),
            "ArrowUp" => Some(Nav(NavAction::SwitchSheet(-1))),
            _ => None,
        };
    }

    // Shift-only (no ctrl, no alt): extend selection.
    if shift && !ctrl && !alt {
        return match key {
            "ArrowRight" => Some(Nav(NavAction::ExpandSelection(Right))),
            "ArrowLeft" => Some(Nav(NavAction::ExpandSelection(Left))),
            "ArrowUp" => Some(Nav(NavAction::ExpandSelection(Up))),
            "ArrowDown" => Some(Nav(NavAction::ExpandSelection(Down))),
            "Tab" => Some(Nav(NavAction::Arrow(Left))),
            _ => None,
        };
    }

    // Any remaining modifier combination is not handled here.
    if ctrl || alt {
        return None;
    }

    // Plain keys - no modifiers.
    match key {
        "ArrowRight" | "Tab" => Some(Nav(NavAction::Arrow(Right))),
        "ArrowLeft" => Some(Nav(NavAction::Arrow(Left))),
        "ArrowDown" | "Enter" => Some(Nav(NavAction::Arrow(Down))),
        "ArrowUp" => Some(Nav(NavAction::Arrow(Up))),
        "PageDown" => Some(Nav(NavAction::PageDown)),
        "PageUp" => Some(Nav(NavAction::PageUp)),
        "Home" => Some(Nav(NavAction::RowHome)),
        "End" => Some(Nav(NavAction::RowEnd)),
        "Delete" => Some(Structure(StructAction::Delete)),
        "Escape" => Some(Edit(EditAction::Cancel)),
        "F2" => Some(Edit(EditAction::EnterEditMode)),
        k if is_printable(k) => Some(Edit(EditAction::Start(k.to_owned()))),
        _ => None,
    }
}

/// True for single printable characters that should start a cell edit.
fn is_printable(key: &str) -> bool {
    key.chars().count() == 1 && key.as_bytes()[0] >= 0x20
}

// Action execution

/// Apply a `SpreadsheetAction` to the model and reactive state.
///
/// Dispatches to category-specific execute functions. Clipboard actions
/// are no-ops here - they require the `AppClipboard` store and async OS
/// clipboard APIs, so the Workbook component handles them directly.
pub fn execute(action: &SpreadsheetAction, model: ModelStore, state: &WorkbookState) {
    let mutates = matches!(
        action,
        SpreadsheetAction::Format(_) | SpreadsheetAction::Structure(_)
    );

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
    if let Err(msg) = result {
        web_sys::console::warn_1(&format!("[RustyCalc] {msg}").into());
    }

    if mutates {
        if let Some(uuid) = state.current_uuid.get_untracked() {
            model.with_value(|m| storage::save(&uuid, m));
        }
    }
}

// Convenience constructors
// Used by the toolbar and other components to avoid deep nesting like
// `SpreadsheetAction::Format(FormatAction::ToggleBold)`.

impl SpreadsheetAction {
    #[cfg(test)]
    pub fn navigate(dir: ArrowKey) -> Self {
        Self::Nav(NavAction::Arrow(dir))
    }
    #[cfg(test)]
    pub fn start_edit(text: String) -> Self {
        Self::Edit(EditAction::Start(text))
    }
    #[cfg(test)]
    pub fn commit(dir: ArrowKey) -> Self {
        Self::Edit(EditAction::CommitAndNavigate(dir))
    }
    pub fn toggle_bold() -> Self {
        Self::Format(FormatAction::ToggleBold)
    }
    pub fn toggle_italic() -> Self {
        Self::Format(FormatAction::ToggleItalic)
    }
    pub fn toggle_underline() -> Self {
        Self::Format(FormatAction::ToggleUnderline)
    }
    pub fn toggle_strikethrough() -> Self {
        Self::Format(FormatAction::ToggleStrikethrough)
    }
    pub fn set_font_size(size: f64) -> Self {
        Self::Format(FormatAction::SetFontSize(size))
    }
    pub fn set_font_family(family: SafeFontFamily) -> Self {
        Self::Format(FormatAction::SetFontFamily(family))
    }
    pub fn set_text_color(hex: HexColor) -> Self {
        Self::Format(FormatAction::SetTextColor(hex))
    }
    pub fn set_background_color(hex: HexColor) -> Self {
        Self::Format(FormatAction::SetBackgroundColor(hex))
    }
    pub fn undo() -> Self {
        Self::Structure(StructAction::Undo)
    }
    pub fn redo() -> Self {
        Self::Structure(StructAction::Redo)
    }
}

// Tests

/// Test-only constructor shortcuts so test call sites don't repeat struct literals.
#[cfg(test)]
impl KeyMod {
    pub fn none() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
        }
    }
    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            shift: false,
            alt: false,
        }
    }
    pub fn shift() -> Self {
        Self {
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
    pub fn alt() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: true,
        }
    }
    pub fn ctrl_shift() -> Self {
        Self {
            ctrl: true,
            shift: true,
            alt: false,
        }
    }
    pub fn ctrl_alt() -> Self {
        Self {
            ctrl: true,
            shift: false,
            alt: true,
        }
    }
    pub fn ctrl_shift_alt() -> Self {
        Self {
            ctrl: true,
            shift: true,
            alt: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CellAddress;
    use crate::model::{mutate, ArrowKey, EvaluationMode};
    use crate::state::{DragState, EditFocus, EditMode, EditingCell};
    use leptos::prelude::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    fn accept_cell() -> EditingCell {
        EditingCell {
            address: CellAddress {
                sheet: 1,
                row: 1,
                column: 1,
            },
            text: String::new(),
            mode: EditMode::Accept,
            focus: EditFocus::Cell,
            text_dirty: false,
        }
    }

    fn edit_cell() -> EditingCell {
        EditingCell {
            address: CellAddress {
                sheet: 1,
                row: 1,
                column: 1,
            },
            text: String::new(),
            mode: EditMode::Edit,
            focus: EditFocus::Cell,
            text_dirty: false,
        }
    }

    // Shorthand for test assertions - avoids `SpreadsheetAction::Nav(NavAction::...)` verbosity.
    fn nav(a: NavAction) -> SpreadsheetAction {
        SpreadsheetAction::Nav(a)
    }
    fn edit(a: EditAction) -> SpreadsheetAction {
        SpreadsheetAction::Edit(a)
    }
    fn fmt(a: FormatAction) -> SpreadsheetAction {
        SpreadsheetAction::Format(a)
    }
    fn struc(a: StructAction) -> SpreadsheetAction {
        SpreadsheetAction::Structure(a)
    }

    /// Returns `((min_row, max_row), (min_col, max_col))` from a `[r1,c1,r2,c2]` range.
    fn selection_bounds(range: [i32; 4]) -> ((i32, i32), (i32, i32)) {
        let [r1, c1, r2, c2] = range;
        ((r1.min(r2), r1.max(r2)), (c1.min(c2), c1.max(c2)))
    }

    // classify_key: plain keys (not editing)

    #[wasm_bindgen_test]
    fn plain_arrows_navigate() {
        let ck = |k| classify_key(k, KeyMod::none(), None);
        assert_eq!(
            ck("ArrowRight"),
            Some(nav(NavAction::Arrow(ArrowKey::Right)))
        );
        assert_eq!(ck("ArrowLeft"), Some(nav(NavAction::Arrow(ArrowKey::Left))));
        assert_eq!(ck("ArrowDown"), Some(nav(NavAction::Arrow(ArrowKey::Down))));
        assert_eq!(ck("ArrowUp"), Some(nav(NavAction::Arrow(ArrowKey::Up))));
    }

    #[wasm_bindgen_test]
    fn tab_navigates_right() {
        assert_eq!(
            classify_key("Tab", KeyMod::none(), None),
            Some(nav(NavAction::Arrow(ArrowKey::Right)))
        );
    }

    #[wasm_bindgen_test]
    fn shift_tab_navigates_left() {
        assert_eq!(
            classify_key("Tab", KeyMod::shift(), None),
            Some(nav(NavAction::Arrow(ArrowKey::Left)))
        );
    }

    #[wasm_bindgen_test]
    fn enter_navigates_down() {
        assert_eq!(
            classify_key("Enter", KeyMod::none(), None),
            Some(nav(NavAction::Arrow(ArrowKey::Down)))
        );
    }

    #[wasm_bindgen_test]
    fn page_up_down() {
        assert_eq!(
            classify_key("PageDown", KeyMod::none(), None),
            Some(nav(NavAction::PageDown))
        );
        assert_eq!(
            classify_key("PageUp", KeyMod::none(), None),
            Some(nav(NavAction::PageUp))
        );
    }

    #[wasm_bindgen_test]
    fn home_end() {
        assert_eq!(
            classify_key("Home", KeyMod::none(), None),
            Some(nav(NavAction::RowHome))
        );
        assert_eq!(
            classify_key("End", KeyMod::none(), None),
            Some(nav(NavAction::RowEnd))
        );
    }

    #[wasm_bindgen_test]
    fn delete_and_escape() {
        assert_eq!(
            classify_key("Delete", KeyMod::none(), None),
            Some(struc(StructAction::Delete))
        );
        assert_eq!(
            classify_key("Escape", KeyMod::none(), None),
            Some(edit(EditAction::Cancel))
        );
    }

    #[wasm_bindgen_test]
    fn f2_enters_edit_mode() {
        assert_eq!(
            classify_key("F2", KeyMod::none(), None),
            Some(edit(EditAction::EnterEditMode))
        );
    }

    #[wasm_bindgen_test]
    fn printable_chars_start_edit() {
        let start = |k: &str| Some(edit(EditAction::Start(k.to_owned())));
        assert_eq!(classify_key("a", KeyMod::none(), None), start("a"));
        assert_eq!(classify_key("=", KeyMod::none(), None), start("="));
        assert_eq!(classify_key("1", KeyMod::none(), None), start("1"));
        assert_eq!(classify_key(" ", KeyMod::none(), None), start(" "));
    }

    #[wasm_bindgen_test]
    fn non_printable_returns_none() {
        let none = |k| classify_key(k, KeyMod::none(), None);
        assert_eq!(none("F1"), None);
        assert_eq!(none("Shift"), None);
        assert_eq!(none("Control"), None);
        assert_eq!(none("Backspace"), None);
        assert_eq!(none("Alt"), None);
    }

    // classify_key: Ctrl combos

    #[wasm_bindgen_test]
    fn ctrl_z_y_undo_redo() {
        let c = |k| classify_key(k, KeyMod::ctrl(), None);
        assert_eq!(c("z"), Some(struc(StructAction::Undo)));
        assert_eq!(c("Z"), Some(struc(StructAction::Undo)));
        assert_eq!(c("y"), Some(struc(StructAction::Redo)));
        assert_eq!(c("Y"), Some(struc(StructAction::Redo)));
    }

    #[wasm_bindgen_test]
    fn ctrl_a_selects_all() {
        assert_eq!(
            classify_key("a", KeyMod::ctrl(), None),
            Some(nav(NavAction::SelectAll))
        );
        assert_eq!(
            classify_key("A", KeyMod::ctrl(), None),
            Some(nav(NavAction::SelectAll))
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_c_x_v_clipboard() {
        let c = |k| classify_key(k, KeyMod::ctrl(), None);
        assert_eq!(c("c"), Some(SpreadsheetAction::Copy));
        assert_eq!(c("C"), Some(SpreadsheetAction::Copy));
        assert_eq!(c("x"), Some(SpreadsheetAction::Cut));
        assert_eq!(c("X"), Some(SpreadsheetAction::Cut));
        assert_eq!(c("v"), Some(SpreadsheetAction::Paste));
        assert_eq!(c("V"), Some(SpreadsheetAction::Paste));
    }

    #[wasm_bindgen_test]
    fn ctrl_b_i_u_formatting() {
        let c = |k| classify_key(k, KeyMod::ctrl(), None);
        assert_eq!(c("b"), Some(fmt(FormatAction::ToggleBold)));
        assert_eq!(c("B"), Some(fmt(FormatAction::ToggleBold)));
        assert_eq!(c("i"), Some(fmt(FormatAction::ToggleItalic)));
        assert_eq!(c("I"), Some(fmt(FormatAction::ToggleItalic)));
        assert_eq!(c("u"), Some(fmt(FormatAction::ToggleUnderline)));
        assert_eq!(c("U"), Some(fmt(FormatAction::ToggleUnderline)));
    }

    #[wasm_bindgen_test]
    fn ctrl_home_end_jump() {
        assert_eq!(
            classify_key("Home", KeyMod::ctrl(), None),
            Some(nav(NavAction::JumpToA1))
        );
        assert_eq!(
            classify_key("End", KeyMod::ctrl(), None),
            Some(nav(NavAction::JumpToLastCell))
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_arrows_navigate_to_edge() {
        let c = |k| classify_key(k, KeyMod::ctrl(), None);
        assert_eq!(c("ArrowRight"), Some(nav(NavAction::Edge(ArrowKey::Right))));
        assert_eq!(c("ArrowLeft"), Some(nav(NavAction::Edge(ArrowKey::Left))));
        assert_eq!(c("ArrowUp"), Some(nav(NavAction::Edge(ArrowKey::Up))));
        assert_eq!(c("ArrowDown"), Some(nav(NavAction::Edge(ArrowKey::Down))));
    }

    #[wasm_bindgen_test]
    fn ctrl_minus_deletes_rows() {
        assert_eq!(
            classify_key("-", KeyMod::ctrl(), None),
            Some(struc(StructAction::DeleteRows))
        );
    }

    // classify_key: Ctrl+Shift combos

    #[wasm_bindgen_test]
    fn ctrl_shift_delete_clears_all() {
        assert_eq!(
            classify_key("Delete", KeyMod::ctrl_shift(), None),
            Some(struc(StructAction::ClearAll))
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_plus_inserts_rows() {
        assert_eq!(
            classify_key("=", KeyMod::ctrl_shift(), None),
            Some(struc(StructAction::InsertRows))
        );
        assert_eq!(
            classify_key("+", KeyMod::ctrl_shift(), None),
            Some(struc(StructAction::InsertRows))
        );
    }

    // classify_key: Ctrl+Alt and Ctrl+Shift+Alt

    #[wasm_bindgen_test]
    fn ctrl_alt_minus_deletes_columns() {
        assert_eq!(
            classify_key("-", KeyMod::ctrl_alt(), None),
            Some(struc(StructAction::DeleteColumns))
        );
    }

    #[wasm_bindgen_test]
    fn ctrl_shift_alt_plus_inserts_columns() {
        assert_eq!(
            classify_key("=", KeyMod::ctrl_shift_alt(), None),
            Some(struc(StructAction::InsertColumns))
        );
        assert_eq!(
            classify_key("+", KeyMod::ctrl_shift_alt(), None),
            Some(struc(StructAction::InsertColumns))
        );
    }

    // classify_key: Alt-only

    #[wasm_bindgen_test]
    fn alt_arrows_switch_sheet() {
        assert_eq!(
            classify_key("ArrowDown", KeyMod::alt(), None),
            Some(nav(NavAction::SwitchSheet(1)))
        );
        assert_eq!(
            classify_key("ArrowUp", KeyMod::alt(), None),
            Some(nav(NavAction::SwitchSheet(-1)))
        );
    }

    // classify_key: Shift-only

    #[wasm_bindgen_test]
    fn shift_arrows_expand_selection() {
        let s = |k| classify_key(k, KeyMod::shift(), None);
        assert_eq!(
            s("ArrowRight"),
            Some(nav(NavAction::ExpandSelection(ArrowKey::Right)))
        );
        assert_eq!(
            s("ArrowLeft"),
            Some(nav(NavAction::ExpandSelection(ArrowKey::Left)))
        );
        assert_eq!(
            s("ArrowUp"),
            Some(nav(NavAction::ExpandSelection(ArrowKey::Up)))
        );
        assert_eq!(
            s("ArrowDown"),
            Some(nav(NavAction::ExpandSelection(ArrowKey::Down)))
        );
    }

    // classify_key: while editing (Accept mode)

    #[wasm_bindgen_test]
    fn accept_mode_enter_tab_commit() {
        let e = accept_cell();
        assert_eq!(
            classify_key("Enter", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Down)))
        );
        assert_eq!(
            classify_key("Tab", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Right)))
        );
        assert_eq!(
            classify_key("Tab", KeyMod::shift(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Left)))
        );
    }

    #[wasm_bindgen_test]
    fn accept_mode_escape_cancels() {
        let e = accept_cell();
        assert_eq!(
            classify_key("Escape", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::Cancel))
        );
    }

    #[wasm_bindgen_test]
    fn accept_mode_arrows_commit_and_navigate() {
        let e = accept_cell();
        assert_eq!(
            classify_key("ArrowDown", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Down)))
        );
        assert_eq!(
            classify_key("ArrowUp", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Up)))
        );
        assert_eq!(
            classify_key("ArrowLeft", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Left)))
        );
        assert_eq!(
            classify_key("ArrowRight", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Right)))
        );
    }

    // classify_key: while editing (Edit mode)

    #[wasm_bindgen_test]
    fn edit_mode_arrows_are_unhandled() {
        let e = edit_cell();
        assert_eq!(classify_key("ArrowDown", KeyMod::none(), Some(&e)), None);
        assert_eq!(classify_key("ArrowUp", KeyMod::none(), Some(&e)), None);
        assert_eq!(classify_key("ArrowLeft", KeyMod::none(), Some(&e)), None);
        assert_eq!(classify_key("ArrowRight", KeyMod::none(), Some(&e)), None);
    }

    #[wasm_bindgen_test]
    fn edit_mode_enter_and_escape_still_work() {
        let e = edit_cell();
        assert_eq!(
            classify_key("Enter", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::CommitAndNavigate(ArrowKey::Down)))
        );
        assert_eq!(
            classify_key("Escape", KeyMod::none(), Some(&e)),
            Some(edit(EditAction::Cancel))
        );
    }

    #[wasm_bindgen_test]
    fn editing_mode_ctrl_c_returns_none() {
        let e = edit_cell();
        assert_eq!(classify_key("c", KeyMod::ctrl(), Some(&e)), None);
        assert_eq!(classify_key("v", KeyMod::ctrl(), Some(&e)), None);
        assert_eq!(classify_key("z", KeyMod::ctrl(), Some(&e)), None);
        assert_eq!(classify_key("a", KeyMod::ctrl(), Some(&e)), None);
    }

    // selection_bounds

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

    // Test setup helper to reduce boilerplate
    #[allow(clippy::unwrap_used)]
    #[cfg(test)]
    fn test_harness() -> (
        StoredValue<ironcalc_base::UserModel<'static>, LocalStorage>,
        WorkbookState,
    ) {
        (
            StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            ),
            crate::state::WorkbookState::new(),
        )
    }

    // execute: navigation

    #[wasm_bindgen_test]
    fn execute_navigate_down_advances_row() {
        let owner = Owner::new();
        owner.with(|| {
            let (model, state) = test_harness();
            execute(&SpreadsheetAction::navigate(ArrowKey::Down), model, &state);
            let row = model.with_value(|m| m.get_selected_view().row);
            assert_eq!(row, 2);
        });
    }

    #[wasm_bindgen_test]
    fn execute_navigate_right_advances_column() {
        let owner = Owner::new();
        owner.with(|| {
            let (model, state) = test_harness();
            execute(&SpreadsheetAction::navigate(ArrowKey::Right), model, &state);
            let col = model.with_value(|m| m.get_selected_view().column);
            assert_eq!(col, 2);
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_jump_to_a1_resets_position() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(&SpreadsheetAction::navigate(ArrowKey::Down), model, &state);
            execute(&SpreadsheetAction::navigate(ArrowKey::Right), model, &state);
            execute(&SpreadsheetAction::Nav(NavAction::JumpToA1), model, &state);
            let v = model.with_value(|m| m.get_selected_view());
            assert_eq!((v.row, v.column), (1, 1));
        });
    }

    // execute: editing
    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_start_edit_sets_editing_cell() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::start_edit("=SUM".to_owned()),
                model,
                &state,
            );
            let cell = state.editing_cell.get_untracked();
            assert!(cell.is_some());
            assert_eq!(cell.unwrap().text, "=SUM");
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_cancel_edit_clears_editing_state() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::start_edit("hello".to_owned()),
                model,
                &state,
            );
            assert!(state.editing_cell.get_untracked().is_some());
            execute(&SpreadsheetAction::Edit(EditAction::Cancel), model, &state);
            assert!(state.editing_cell.get_untracked().is_none());
            assert!(!matches!(
                state.drag.get_untracked(),
                DragState::Pointing { .. }
            ));
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_commit_writes_value_and_navigates() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            execute(
                &SpreadsheetAction::start_edit("42".to_owned()),
                model,
                &state,
            );
            execute(&SpreadsheetAction::commit(ArrowKey::Down), model, &state);
            let val = model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            assert_eq!(val, "42");
            assert!(state.editing_cell.get_untracked().is_none());
            let row = model.with_value(|m| m.get_selected_view().row);
            assert_eq!(row, 2);
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_enter_edit_mode_loads_existing_content() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();

            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 1, 1, "hello").ok();
            });
            execute(
                &SpreadsheetAction::Edit(EditAction::EnterEditMode),
                model,
                &state,
            );
            let cell = state.editing_cell.get_untracked().unwrap();
            assert_eq!(cell.mode, EditMode::Edit);
            assert_eq!(cell.text, "hello");
        });
    }

    // execute: mutations
    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_delete_clears_cell_content() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 1, 1, "data").ok();
            });
            execute(&struc(StructAction::Delete), model, &state);
            let val = model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            assert_eq!(val, "");
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_undo_redo_roundtrip() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 1, 1, "42").ok();
            });
            execute(&SpreadsheetAction::undo(), model, &state);
            let after_undo =
                model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            assert_eq!(after_undo, "");
            execute(&SpreadsheetAction::redo(), model, &state);
            let after_redo =
                model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            assert_eq!(after_redo, "42");
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_insert_row_pushes_content_down() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 1, 1, "original").ok();
            });
            execute(&struc(StructAction::InsertRows), model, &state);
            let a1 = model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            let a2 = model.with_value(|m| m.get_formatted_cell_value(0, 2, 1).unwrap_or_default());
            assert_eq!(a1, "");
            assert_eq!(a2, "original");
        });
    }

    #[allow(clippy::unwrap_used)]
    #[wasm_bindgen_test]
    fn execute_delete_row_pulls_content_up() {
        let owner = Owner::new();
        owner.with(|| {
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
            );
            let state = crate::state::WorkbookState::new();
            mutate(model, EvaluationMode::Immediate, |m| {
                m.set_user_input(0, 2, 1, "data").ok();
            });
            execute(&struc(StructAction::DeleteRows), model, &state);
            let a1 = model.with_value(|m| m.get_formatted_cell_value(0, 1, 1).unwrap_or_default());
            assert_eq!(a1, "data");
        });
    }
}
