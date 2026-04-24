//! Shared wiring between text editors (formula bar, in-cell overlay) and
//! the [`EditingCell`] signal.
//!
//! Both editor sites agree on one invariant: every keystroke must atomically
//! update `text`, `cursor`, `text_dirty`, and `formula_analysis` in lockstep.
//! Workbook's point-mode router reads `cursor` on every arrow key
//! (`components/workbook.rs` on_keydown), so drift here breaks reference
//! splicing. These helpers are the single authoritative place that upholds
//! that invariant — the editor components are thin DOM wrappers around them.
//!
//! Policy (when to *start* an edit session, or how to focus) stays with the
//! caller. These helpers only care about syncing state that already exists.

use wasm_bindgen::JsCast;

use crate::input::formula_analysis::analyze_formula;
use crate::state::{EditingCell, Split};

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

/// Mirror a keystroke into the active [`EditingCell`] signal.
///
/// No-ops when no session exists — the caller owns the decision to start one
/// (e.g. formula bar's first-keystroke Accept path). Keeping that policy out
/// here means both editor sites can share the body without coupling.
pub fn sync_edit(
    editing: Split<Option<EditingCell>>,
    value: String,
    cursor: usize,
    sheet_names: &[(u32, String)],
    defined_names: &[crate::coord::DefinedName],
) {
    editing.update(|cell| {
        if let Some(c) = cell {
            c.formula_analysis = analyze_formula(&value, c.address, sheet_names, defined_names);
            c.text = value;
            c.text_dirty = true;
            c.cursor = cursor;
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
