//! Pure key + modifier classification — maps a keyboard event to a
//! [`SpreadsheetAction`], with no DOM access and no side effects.
//!
//! This is the file new contributors touch when adding a shortcut. Keep it
//! free of dispatch glue and state — both belong in
//! [`super::dispatch`].

use crate::input::{
    edit::EditAction, format::FormatAction, nav::NavAction, structure::StructAction,
};
use crate::model::ArrowKey;
use crate::state::{EditMode, EditingCell};

use super::action::SpreadsheetAction;

/// Keyboard modifier state at the time of a key event.
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
            "Enter" if ctrl && shift => Some(Edit(EditAction::CommitArrayAndNavigate(Down))),
            // NOTE: Alt+Enter (newline insert) is handled in the workbook keydown
            // handler *before* classify — it needs the DOM caret. It never
            // reaches here.
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
    // Only known navigation keys with Shift are consumed here.
    // Shift+letter (e.g., Shift+A = "A") and other printable combos
    // fall through to the is_printable check below so they start a
    // cell edit with the capital letter.
    if shift
        && !ctrl
        && !alt
        && let Some(action) = match key {
            "ArrowRight" => Some(Nav(NavAction::ExpandSelection(Right))),
            "ArrowLeft" => Some(Nav(NavAction::ExpandSelection(Left))),
            "ArrowUp" => Some(Nav(NavAction::ExpandSelection(Up))),
            "ArrowDown" => Some(Nav(NavAction::ExpandSelection(Down))),
            "Tab" => Some(Nav(NavAction::Arrow(Left))),
            _ => None,
        }
    {
        return Some(action);
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
