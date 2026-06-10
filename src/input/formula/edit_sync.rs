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

use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::coord::CellAddress;
use crate::model::SheetRoster;
use crate::model::frontend_model::DefinedNameManager;
use crate::state::{EditingCell, EditingDefinedName, ModelStore, Split};

use super::analysis::{FormulaAnalysis, analyze_formula};

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

/// Alt+Enter: splice a literal newline at the caret of the active cell edit.
///
/// Browsers insert `\n` natively only for plain / Shift+Enter, never Alt+Enter,
/// so we do it ourselves: rebuild the buffer with `\n` at the caret and push it
/// through [`sync_edit`] (text + cursor + analysis update in lockstep). The
/// editor is a controlled `prop:value`, so the re-render snaps the caret to the
/// end — we restore it on the next tick via `set_timeout(0)`, after Leptos has
/// flushed the new value to the DOM.
pub fn insert_newline_at_caret(
    editing: Split<Option<EditingCell>>,
    model: ModelStore,
    target: &web_sys::EventTarget,
) {
    let Some((value, raw_cursor)) = read_value_and_cursor(target) else {
        return;
    };
    // `raw_cursor` is a UTF-16 code-unit offset (the DOM's unit), but Rust
    // slices `value` by UTF-8 byte. Convert before slicing — otherwise a
    // multibyte char shifts the split point: in "SALES DASHBOARD — FY 2026"
    // the em-dash is 1 UTF-16 unit but 3 bytes, so a caret at the end (offset
    // 25) would slice at byte 25 and split "2026" into "20\n26".
    let at = utf16_offset_to_byte(&value, raw_cursor);

    let mut new_text = String::with_capacity(value.len() + 1);
    new_text.push_str(&value[..at]);
    new_text.push('\n');
    new_text.push_str(&value[at..]);
    // The caret round-trips back through the DOM (sync_edit + set_selection_range),
    // which counts UTF-16 units, and '\n' is one unit — advance the UTF-16
    // offset, not the byte offset.
    let new_cursor = raw_cursor + 1;

    let sheet_names = model.with_value(|m| m.get_sheet_names());
    let defined_names = model.with_value(|m| m.get_defined_names());
    sync_edit(editing, new_text, new_cursor, &sheet_names, &defined_names);

    // Restore the caret after the controlled re-render resets it to the end.
    if let Some(ta) = target.dyn_ref::<web_sys::HtmlTextAreaElement>() {
        let ta = ta.clone();
        set_timeout(
            move || {
                let _ = ta.set_selection_range(new_cursor as u32, new_cursor as u32);
            },
            Duration::from_millis(0),
        );
    }
}

/// Convert a UTF-16 code-unit offset (as the DOM reports via `selectionEnd`)
/// into a UTF-8 byte offset into `s` (as Rust string slicing requires).
///
/// Walk `s` one `char` at a time, accumulating each char's UTF-16 length
/// ([`char::len_utf16`] — 1 for the BMP, 2 for astral chars) until the running
/// total reaches `utf16_off`; the byte index at that point is the answer.
/// Offsets at or past the end clamp to `s.len()`; one that would land
/// mid-surrogate-pair resolves to the boundary just before it, since surrogate
/// halves can't be addressed in UTF-8 anyway.
fn utf16_offset_to_byte(s: &str, utf16_off: usize) -> usize {
    let mut u16_count = 0;
    for (byte_idx, ch) in s.char_indices() {
        // Check before consuming this char: once the running UTF-16 count has
        // reached the target, this char's byte index is where the offset lands.
        if u16_count >= utf16_off {
            return byte_idx;
        }
        u16_count += ch.len_utf16();
    }
    // Offset at or past the end of the string.
    s.len()
}

#[cfg(test)]
mod tests {
    use super::utf16_offset_to_byte;

    // Pure ASCII: 1 byte == 1 UTF-16 unit, so the offset passes through.
    #[test]
    fn ascii_offset_is_identity() {
        assert_eq!(utf16_offset_to_byte("FY 2026", 3), 3);
    }

    // The reported bug: the em-dash is 1 UTF-16 unit but 3 UTF-8 bytes, so a
    // caret at the end (UTF-16 offset 25) must map to byte 27 — not byte 25,
    // which splits "2026" into "20" / "26".
    #[test]
    fn caret_after_em_dash_maps_past_its_extra_bytes() {
        let s = "SALES DASHBOARD — FY 2026";
        assert_eq!(s.chars().count(), 25);
        assert_eq!(s.len(), 27);
        assert_eq!(utf16_offset_to_byte(s, 25), 27);
        // Caret right after the em-dash (UTF-16 offset 17) -> just past its
        // 3 bytes (byte 19), so the space that follows stays intact.
        assert_eq!(utf16_offset_to_byte(s, 17), 19);
    }

    // Past-the-end offset clamps to the byte length rather than panicking.
    #[test]
    fn offset_past_end_clamps() {
        assert_eq!(utf16_offset_to_byte("ab", 99), 2);
    }
}
