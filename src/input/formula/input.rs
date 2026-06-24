//! Pure helpers for formula point-mode editing.
//!
//! These operate on formula strings and cursor positions; they have no side
//! effects and do not touch the model.

use crate::coord::{CellAddress, PointingStep, RefNode, SheetRange, TextRef};
use crate::model::ArrowKey;

use super::ref_mode::is_in_reference_mode;

// In-formula reference splicing

/// Replace or insert a reference string inside a formula.
///
/// `span` marks the region to replace: pass an existing `RefSpan` to overwrite
/// a previous reference, or `RefSpan::at(cursor)` to insert at the cursor position.
///
/// Returns `(new_text, new_span)` so the caller can store the span and replace
/// it again on the next arrow keypress or cell click.
pub fn splice_ref(text: &str, span: TextRef, ref_str: &str) -> (String, TextRef) {
    // Guard against out-of-range spans (e.g. after the user typed extra chars).
    let start = span.start.min(text.len());
    let end = span.end.min(text.len()).max(start);
    let new_text = format!("{}{}{}", &text[..start], ref_str, &text[end..]);
    let new_end = start + ref_str.len();
    (
        new_text,
        TextRef {
            start,
            end: new_end,
        },
    )
}

/// Pure splice for a formula-ref drag drop. Builds the new ref text via
/// `RefNode::with_area` (so `$`-flags and `Sheet!` prefix survive) and
/// splices it into the formula at `span`. Returns `None` when the new
/// range equals the original ref's resolved area — Excel's drop-on-
/// origin silence — so callers can skip the no-op edit.
pub fn splice_dragged_ref(
    text: &str,
    span: TextRef,
    original_ref: &RefNode,
    new_range: SheetRange,
    editing: CellAddress,
) -> Option<(String, TextRef)> {
    if original_ref.area(&editing) == new_range {
        return None;
    }
    let new_node = original_ref.with_area(new_range, editing);
    let new_str = new_node.to_localized(&editing.as_stringify_ctx());
    Some(splice_ref(text, span, &new_str))
}

/// All state needed to evaluate a point-mode keypress, drawn from `EditingCell` and `DragState`.
///
/// Passed to `try_point_move` so callers don't manage 6 separate parameters.
pub struct PointMoveCtx<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub already_pointing: bool,
    /// Current point-mode reference — carried as a `RefNode` so absolute flags,
    /// sheet qualification, and cross-sheet names survive arrow-key moves.
    pub current_ref: RefNode,
    pub prev_span: Option<TextRef>,
    /// The cell being edited — feeds ironcalc's stringify ctx for relative
    /// ref delta resolution (`row`/`column` fields are offsets when the
    /// matching `absolute_*` flag is false).
    pub editing: CellAddress,
}

/// Outcome of evaluating a keypress in point mode.
///
/// Returned by `try_point_move` so callers handle all three paths in a single `match`,
/// instead of calling `should_exit_pointing` and `try_point_move` separately.
#[derive(Debug, PartialEq)]
pub enum PointMoveOutcome {
    /// Modifier-only key (Shift/Ctrl/Alt/Meta), or arrow key not at a reference insertion point.
    NoAction,
    /// Non-arrow, non-modifier key while pointing — caller should set `DragState::Idle`.
    ExitPointing,
    /// Arrow key moved the point selection. Caller applies the result.
    Move(PointingStep),
}

/// Evaluate a point-mode keypress from pure inputs.
///
/// # Caller responsibilities
///
/// - Gate on `may_point` (`edit.mode == Accept || edit.text_dirty || already_pointing`)
///   before calling — this involves `EditMode` (UI state), not formula text.
/// - Guard against Ctrl/Alt modifiers before calling — they suppress point mode.
/// - Apply signal writes from the returned [`PointMoveOutcome`].
pub fn try_point_move(ctx: &PointMoveCtx, key: &str, is_shift: bool) -> PointMoveOutcome {
    // Modifier-only keys never change point state.
    if matches!(key, "Shift" | "Control" | "Alt" | "Meta") {
        return PointMoveOutcome::NoAction;
    }

    // Any non-arrow key signals the user is done pointing (e.g. typed an operator or digit).
    if ArrowKey::from_str(key).is_none() {
        return PointMoveOutcome::ExitPointing;
    }

    // Allow entry when the caller seeded prev_span from a caret-hit, even if
    // `is_in_reference_mode` says no (the cursor is INSIDE a ref, not at an
    // insertion point — a different path to the same Point outcome).
    if !ctx.already_pointing
        && ctx.prev_span.is_none()
        && !is_in_reference_mode(ctx.text, ctx.cursor)
    {
        return PointMoveOutcome::NoAction;
    }

    if let Some(arrow) = ArrowKey::from_str(key) {
        let new_ref = if is_shift {
            ctx.current_ref.extend_with_anchor(&arrow)
        } else {
            ctx.current_ref.extend_trailing(&arrow)
        };

        let ref_str = new_ref.to_localized(&ctx.editing.as_stringify_ctx());

        let (new_text, new_span) = splice_ref(
            ctx.text,
            ctx.prev_span.unwrap_or(TextRef::at(ctx.cursor)),
            &ref_str,
        );

        PointMoveOutcome::Move(PointingStep {
            text: new_text,
            ref_node: new_ref,
            span: new_span,
        })
    } else {
        PointMoveOutcome::ExitPointing
    }
}
