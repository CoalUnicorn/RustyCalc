//! In-progress edit of a row in the "Manage Named Ranges" dialog.

use crate::coord::CellAddress;
use crate::input::formula::FormulaAnalysis;

/// In-progress edit of a row in the Manage Named Ranges dialog.
///
/// Slim shape: every field is load-bearing. Compare with
/// [`super::editing_cell::EditingCell`], which carries `mode` / `focus` /
/// `text_dirty` / a real `address` because it lives inside the canvas's
/// keyboard router. The dialog has none of those concerns (no point-mode,
/// no focus arbitration with the canvas), so those fields would be dead
/// weight here.
///
/// `sync_edit` works for both kinds of edit via the
/// `FormulaEditState` trait.
#[derive(Clone, Debug, PartialEq)]
pub struct EditingDefinedName {
    /// `None` when creating a new row; `Some((name, scope))` when editing an
    /// existing one. Identifies the row to call `rename_defined_name` against
    /// on save (vs. `create_defined_name` when `None`).
    pub(crate) original: Option<(String, Option<u32>)>,
    pub(crate) name: String,
    pub(crate) scope: Option<u32>,
    /// Formula body without the leading `=`. Stored bare so it round-trips
    /// against ironcalc's `new_defined_name` / `update_defined_name` (both
    /// expect the body, not the `=...` form).
    pub(crate) formula: String,
    /// Cursor position as a UTF-8 byte offset into `formula` (converted from
    /// the DOM's UTF-16 by `sync_edit`).
    pub(crate) cursor: usize,
    pub(crate) formula_analysis: FormulaAnalysis,
    /// Cell whose position interprets relative refs in `formula`. Captured
    /// from the active cell at dialog-open time (Excel's convention) and
    /// frozen for the lifetime of the edit, so toggling sheet tabs behind
    /// the modal can't shift the parser's frame underneath the user.
    pub(crate) context_cell: CellAddress,
}

impl EditingDefinedName {
    /// Formula side of the save gate: an analyzer error, or bare refs under
    /// Workbook scope. Workbook-scoped names need fully-qualified refs
    /// (`Sheet1!A1`) so they round-trip unambiguously regardless of the
    /// active view sheet.
    pub(crate) fn formula_invalid(&self) -> bool {
        self.formula_analysis.has_any_error()
            || (self.scope.is_none() && self.formula_analysis.has_bare_refs())
    }

    /// Full save gate: blank name, or [`Self::formula_invalid`].
    pub(crate) fn save_blockers(&self) -> bool {
        self.name.trim().is_empty() || self.formula_invalid()
    }
}
