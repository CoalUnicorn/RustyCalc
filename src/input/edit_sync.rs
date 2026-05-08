//! Shared wiring between formula text editors (cell editor / formula bar /
//! Manage Named Ranges dialog) and their backing edit-state signals.
//!
//! Every editor site agrees on one invariant: a keystroke must atomically
//! update `text`, `cursor`, and `formula_analysis` in lockstep. Workbook's
//! point-mode router reads `cursor` on every arrow key
//! (`components/workbook.rs` on_keydown), so drift here breaks reference
//! splicing. These helpers are the single authoritative place that upholds
//! that invariant — the editor components are thin DOM wrappers around them.
//!
//! [`FormulaEditState`] abstracts over the two flavours of in-progress edit
//! the project carries today:
//! - [`crate::state::EditingCell`] — a cell edit, also flips `text_dirty`
//!   to arm point-mode.
//! - [`crate::state::EditingDefinedName`] — a row in the Manage Named Ranges
//!   dialog, no point-mode (V1).
//!
//! Policy (when to *start* an edit session, or how to focus) stays with the
//! caller. These helpers only care about syncing state that already exists.

use wasm_bindgen::JsCast;

use crate::coord::CellAddress;
use crate::input::formula_analysis::{analyze_formula, FormulaAnalysis};
use crate::state::{EditingCell, EditingDefinedName, Split};

/// Extract `(value, cursor)` from an input or textarea event target.
///
/// Both `HtmlInputElement` and `HtmlTextAreaElement` expose `value()` and
/// `selection_end()`, so callers don't need to branch on element type.
/// `selection_end` can be `None` when the element hasn't received focus yet —
/// fall back to end-of-text in that case.
///
/// Returns `None` only when the target is neither kind of text field.
//
// NOTE: consider return new type
pub fn read_value_and_cursor(target: &web_sys::EventTarget) -> Option<(String, usize)> {
    if let Some(input) = target.dyn_ref::<web_sys::HtmlInputElement>() {
        let value = input.value();
        let cursor = input
            .selection_end()
            .ok()
            .flatten()
            .map(|n| n as usize)
            .unwrap_or_else(|| value.len());
        return Some((value, cursor));
    }
    if let Some(textarea) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        let value = textarea.value();
        let cursor = textarea
            .selection_end()
            .ok()
            .flatten()
            .map(|n| n as usize)
            .unwrap_or_else(|| value.len());
        return Some((value, cursor));
    }
    None
}

/// What [`sync_edit`] needs from any in-progress formula edit.
///
/// Two methods, no more: `context_cell()` is the address `analyze_formula`
/// uses to interpret relative refs, and `apply_edit()` is the atomic writer
/// each implementor uses to update its own fields. The split lets
/// [`EditingCell`] additionally flip `text_dirty` (to arm point-mode) and
/// lets [`EditingDefinedName`] write to a different field name (`formula`,
/// not `text`) without either implementor leaking through the trait.
pub trait FormulaEditState {
    fn context_cell(&self) -> CellAddress;
    fn apply_edit(&mut self, text: String, cursor: usize, analysis: FormulaAnalysis);
}

impl FormulaEditState for EditingCell {
    fn context_cell(&self) -> CellAddress {
        self.address
    }
    fn apply_edit(&mut self, text: String, cursor: usize, analysis: FormulaAnalysis) {
        // text_dirty arms point-mode for the next arrow keypress — the cell
        // editor is the only edit site where arrows can splice a reference.
        self.text = text;
        self.cursor = cursor;
        self.formula_analysis = analysis;
        self.text_dirty = true;
    }
}

impl FormulaEditState for EditingDefinedName {
    fn context_cell(&self) -> CellAddress {
        self.context_cell
    }
    fn apply_edit(&mut self, text: String, cursor: usize, analysis: FormulaAnalysis) {
        // No `text_dirty`: the dialog has no point-mode in V1, so there's
        // nothing to arm. If/when V2 adds point-mode here, add the flag and
        // mirror the cell-editor branch.
        self.formula = text;
        self.cursor = cursor;
        self.formula_analysis = analysis;
    }
}

/// Mirror a keystroke into any active formula-edit signal.
///
/// No-ops when no session exists — the caller owns the decision to start
/// one (e.g. formula bar's first-keystroke Accept path). Keeping that policy
/// out here means every editor site can share the body without coupling.
pub fn sync_edit<T>(
    editing: Split<Option<T>>,
    value: String,
    cursor: usize,
    sheet_names: &[(u32, String)],
    defined_names: &[crate::coord::DefinedName],
) where
    T: FormulaEditState + Clone + Send + Sync + 'static,
{
    editing.update(|slot| {
        if let Some(c) = slot {
            let analysis = analyze_formula(&value, c.context_cell(), sheet_names, defined_names);
            c.apply_edit(value, cursor, analysis);
        }
    });
}

/// Swallow the browser's default for Enter / Tab / Escape, but let the event
/// bubble. Workbook's `on_keydown` classifier owns commit / cancel / navigate
/// routing — stopping propagation would break that contract.
pub fn suppress_navigation_defaults(ev: &web_sys::KeyboardEvent) {
    if matches!(ev.key().as_str(), "Enter" | "Tab" | "Escape") {
        ev.prevent_default();
    }
}
