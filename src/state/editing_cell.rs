//! In-progress cell-edit state, with arrow-key and focus modes.

use crate::coord::CellAddress;
use crate::input::formula::FormulaAnalysis;

/// Arrow key behavior during a cell edit.
#[derive(Clone, Debug, PartialEq)]
pub enum EditMode {
    /// Arrows commit and navigate. Default from printable keypress.
    Accept,
    /// Arrows move text cursor. Entered via F2 or double-click.
    Edit,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EditFocus {
    Cell,
    FormulaBar,
}

/// In-progress cell edit not yet committed to the model.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingCell {
    pub(crate) address: CellAddress,
    pub(crate) text: String,
    pub(crate) mode: EditMode,
    pub(crate) focus: EditFocus,
    /// Set on user input (typing, paste); cleared on arrow key consumption.
    /// In `Edit` mode, gates whether arrows enter point-mode — distinguishes
    /// "typed an operator" from "cursor moved through a reference position".
    pub(crate) text_dirty: bool,
    /// Cached result of the last `analyze_formula()` call.
    /// Updated synchronously on each `on_input` event in formula_bar and cell_editor.
    pub(crate) formula_analysis: FormulaAnalysis,
    /// Cursor position as a UTF-8 byte offset into `text` — not the DOM's
    /// UTF-16 `selectionEnd`, which `sync_edit` converts. Updated on every
    /// input event.
    pub(crate) cursor: usize,
}
