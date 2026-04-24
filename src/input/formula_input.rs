/// Pure helpers for formula point-mode editing.
///
/// These operate on formula strings and cursor positions; they have no side
/// effects and do not touch the model.
use crate::coord::{Cell, PointingStep, RefNode, TextRef};
use crate::input::formula_analysis::is_in_reference_mode;
use crate::model::ArrowKey;

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

/// All state needed to evaluate a point-mode keypress, drawn from `EditingCell` and `DragState`.
///
/// Passed to `try_point_move` so callers don't manage 8 separate parameters.
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
    pub editing: Cell,
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
    if !ArrowKey::from_str(&key).is_some() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    // splice_ref

    #[wasm_bindgen_test]
    fn splice_insert_at_cursor_no_prev_span() {
        assert_eq!(
            splice_ref("=SUM(", TextRef::at(5), "A1"),
            ("=SUM(A1".to_string(), TextRef { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_prev_span() {
        let rs = TextRef { start: 5, end: 7 };
        assert_eq!(
            splice_ref("=SUM(A1)", rs, "B2"),
            ("=SUM(B2)".to_string(), TextRef { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_insert_after_equals() {
        assert_eq!(
            splice_ref("=", TextRef::at(1), "A1"),
            ("=A1".to_string(), TextRef { start: 1, end: 3 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_span_out_of_range_clamps() {
        // prev_span (10, 15) is beyond text length 3 - clamps to (3, 3) -> append.
        let rs = TextRef { start: 10, end: 15 };
        assert_eq!(
            splice_ref("=A1", rs, "B2"),
            ("=A1B2".to_string(), TextRef { start: 3, end: 5 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_extends_span_when_ref_is_longer() {
        let rs = TextRef { start: 1, end: 3 };
        assert_eq!(
            splice_ref("=A1", rs, "Sheet2!A1:B100"),
            ("=Sheet2!A1:B100".to_string(), TextRef { start: 1, end: 15 })
        );
    }

    // try_point_move
    //
    // All tests use editing=A1 on sheet 1; the RefNode's stored fields are
    // therefore deltas from (1,1). `RefNode::cell(1, None, 0, 0, false, false)`
    // means "sheet 1, single cell, zero offset from editing" → resolves to A1.

    fn editing_a1() -> Cell {
        Cell {
            sheet: 1,
            row: 1,
            column: 1,
        }
    }

    /// `current_ref` defaults to A1 (zero delta from editing=A1) — callers
    /// pointing at other cells override by passing a pre-built `RefNode`.
    fn ctx<'a>(
        text: &'a str,
        cursor: usize,
        already_pointing: bool,
        current_ref: RefNode,
        prev_span: Option<TextRef>,
    ) -> PointMoveCtx<'a> {
        PointMoveCtx {
            text,
            cursor,
            already_pointing,
            current_ref,
            prev_span,
            editing: editing_a1(),
        }
    }

    /// 1x1 range at A1 — zero row/column delta from editing=A1.
    fn at_a1() -> RefNode {
        RefNode::cell(1, None, 0, 0, false, false)
    }

    /// 1x1 range at B3 — row delta 2, column delta 1 from editing=A1.
    fn at_b3() -> RefNode {
        RefNode::cell(1, None, 2, 1, false, false)
    }

    #[wasm_bindgen_test]
    fn point_move_non_arrow_key_exits_pointing() {
        // Enter is a non-modifier non-arrow key — signals the user is done pointing.
        assert_eq!(
            try_point_move(&ctx("=", 1, false, at_a1(), None), "Enter", false),
            PointMoveOutcome::ExitPointing,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_modifier_key_is_no_action() {
        // Shift alone must not exit pointing (user is extending a selection).
        assert_eq!(
            try_point_move(&ctx("=", 1, true, at_a1(), None), "Shift", false),
            PointMoveOutcome::NoAction,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_cursor_after_ref_token_not_pointing_is_no_action() {
        // Cursor at end of "=A1" (position 3) — last char is '1', not an operator.
        // is_in_reference_mode returns false; already_pointing is false → NoAction.
        assert_eq!(
            try_point_move(&ctx("=A1", 3, false, at_a1(), None), "ArrowDown", false),
            PointMoveOutcome::NoAction,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_already_pointing_bypasses_ref_mode_check() {
        // "=A1" cursor at 3 is not in ref mode normally, but already_pointing=true
        // bypasses the is_in_reference_mode guard → Move.
        assert!(matches!(
            try_point_move(
                &ctx("=A1", 3, true, at_a1(), Some(TextRef { start: 1, end: 3 })),
                "ArrowDown",
                false,
            ),
            PointMoveOutcome::Move(_),
        ));
    }

    #[wasm_bindgen_test]
    fn point_move_bare_equals_arrow_down_enters_a2() {
        // "=" cursor=1: is_in_reference_mode returns true (bare equals).
        // ArrowDown from A1 (zero delta) → stored row delta becomes +1, stringifies "A2".
        assert_eq!(
            try_point_move(&ctx("=", 1, false, at_a1(), None), "ArrowDown", false),
            PointMoveOutcome::Move(PointingStep {
                text: "=A2".to_string(),
                ref_node: RefNode::cell(1, None, 1, 0, false, false),
                span: TextRef { start: 1, end: 3 },
            }),
        );
    }

    #[wasm_bindgen_test]
    fn point_move_shift_extends_anchor() {
        // Already pointing at B3 with editing A1: anchor stored as (2,1) delta.
        // Shift+ArrowDown extends trailing only → RangeKind (2,1)..(3,1) → "B3:B4".
        assert_eq!(
            try_point_move(
                &ctx("=B3", 3, true, at_b3(), Some(TextRef { start: 1, end: 3 })),
                "ArrowDown",
                true,
            ),
            PointMoveOutcome::Move(PointingStep {
                text: "=B3:B4".to_string(),
                ref_node: RefNode::range(1, None, 2, 1, false, false, 3, 1, false, false),
                span: TextRef { start: 1, end: 6 },
            }),
        );
    }

    /// Reproducer: user editing C1 with `=$A$1+` at cursor=6 presses ArrowDown.
    /// Expected: point-mode engages, splices `C2` producing `=$A$1+C2`.
    #[test]
    fn point_move_reproduce_abs_then_operator_then_arrow() {
        use crate::coord::SheetRange;
        let editing = Cell {
            sheet: 0,
            row: 1,
            column: 3,
        };
        let area = SheetRange::from_cell(editing.sheet, editing.row, editing.column);
        let current_ref = RefNode::from_cell_area(area, editing, "");
        let ctx = PointMoveCtx {
            text: "=$A$1+",
            cursor: 6,
            already_pointing: false,
            current_ref,
            prev_span: None,
            editing,
        };
        match try_point_move(&ctx, "ArrowDown", false) {
            PointMoveOutcome::Move(step) => {
                assert_eq!(step.text, "=$A$1+C2");
            }
            other => panic!("expected Move, got {:?}", other),
        }
    }

    #[wasm_bindgen_test]
    fn point_move_plain_arrow_moves_whole_range() {
        // Already pointing at B3, plain ArrowRight collapses to trailing+delta:
        // delta (2,1) + (0,1) = (2,2) → stringify C3.
        assert_eq!(
            try_point_move(
                &ctx("=B3", 3, true, at_b3(), Some(TextRef { start: 1, end: 3 })),
                "ArrowRight",
                false,
            ),
            PointMoveOutcome::Move(PointingStep {
                text: "=C3".to_string(),
                ref_node: RefNode::cell(1, None, 2, 2, false, false),
                span: TextRef { start: 1, end: 3 },
            }),
        );
    }

    // Story 2 — "Fix this ref": caret inside an existing resolved ref seeds
    // `prev_span` + `current_ref` from the refs_at_cursor hit. The new guard
    // lets entry through without `is_in_reference_mode` and without
    // `already_pointing`, so arrow keys replace the ref under the caret
    // while preserving its identity.

    #[test]
    fn caret_on_absolute_ref_preserves_dollars() {
        // `=$A$1`, caret between `$A` and `$1`. Not an operator-adjacent
        // insertion point, but prev_span is seeded from the caret-hit →
        // the new guard accepts entry. ArrowDown must emit `$A$2`, not
        // `A2` — both absolute flags survive `extend_trailing`.
        let ctx = PointMoveCtx {
            text: "=$A$1",
            cursor: 3,
            already_pointing: false,
            current_ref: RefNode::cell(1, None, 1, 1, true, true),
            prev_span: Some(TextRef { start: 1, end: 5 }),
            editing: editing_a1(),
        };
        assert_eq!(
            try_point_move(&ctx, "ArrowDown", false),
            PointMoveOutcome::Move(PointingStep {
                text: "=$A$2".to_string(),
                ref_node: RefNode::cell(1, None, 2, 1, true, true),
                span: TextRef { start: 1, end: 5 },
            }),
        );
    }

    #[test]
    fn caret_on_cross_sheet_ref_preserves_sheet() {
        // `=Sheet2!B2`, caret inside `B2` (position 9). With editing=A1 the
        // relative deltas for B2 are (+1,+1). ArrowRight moves the column
        // delta to +2 → absolute C2 on Sheet2. `sheet_name` survives into
        // the new RefNode; `to_localized` re-emits the `Sheet2!` prefix.
        let ctx = PointMoveCtx {
            text: "=Sheet2!B2",
            cursor: 9,
            already_pointing: false,
            current_ref: RefNode::cell(2, Some("Sheet2".to_string()), 1, 1, false, false),
            prev_span: Some(TextRef { start: 1, end: 10 }),
            editing: editing_a1(),
        };
        assert_eq!(
            try_point_move(&ctx, "ArrowRight", false),
            PointMoveOutcome::Move(PointingStep {
                text: "=Sheet2!C2".to_string(),
                ref_node: RefNode::cell(2, Some("Sheet2".to_string()), 1, 2, false, false),
                span: TextRef { start: 1, end: 10 },
            }),
        );
    }

    #[test]
    fn caret_on_relative_ref_stays_relative() {
        // `=A1`, caret between A and 1. Fully relative → no `$` introduced.
        // This is the control case: absence of flags must not cause
        // `extend_trailing` to invent them.
        let ctx = PointMoveCtx {
            text: "=A1",
            cursor: 2,
            already_pointing: false,
            current_ref: RefNode::cell(1, None, 0, 0, false, false),
            prev_span: Some(TextRef { start: 1, end: 3 }),
            editing: editing_a1(),
        };
        assert_eq!(
            try_point_move(&ctx, "ArrowDown", false),
            PointMoveOutcome::Move(PointingStep {
                text: "=A2".to_string(),
                ref_node: RefNode::cell(1, None, 1, 0, false, false),
                span: TextRef { start: 1, end: 3 },
            }),
        );
    }
}
