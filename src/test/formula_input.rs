use crate::coord::{Absolute, CellAddress, PointingStep, RefNode, TextRef};
use crate::input::formula::input::*;
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
// therefore deltas from (1,1). `RefNode::cell(1, None, 0, 0, Absolute { row: false, column: false })`
// means "sheet 1, single cell, zero offset from editing" -> resolves to A1.

fn editing_a1() -> CellAddress {
    CellAddress {
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
    RefNode::cell(
        1,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    )
}

/// 1x1 range at B3 — row delta 2, column delta 1 from editing=A1.
fn at_b3() -> RefNode {
    RefNode::cell(
        1,
        None,
        2,
        1,
        Absolute {
            row: false,
            column: false,
        },
    )
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
    // is_in_reference_mode returns false; already_pointing is false -> NoAction.
    assert_eq!(
        try_point_move(&ctx("=A1", 3, false, at_a1(), None), "ArrowDown", false),
        PointMoveOutcome::NoAction,
    );
}

#[wasm_bindgen_test]
fn point_move_already_pointing_bypasses_ref_mode_check() {
    // "=A1" cursor at 3 is not in ref mode normally, but already_pointing=true
    // bypasses the is_in_reference_mode guard -> Move.
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
    // ArrowDown from A1 (zero delta) -> stored row delta becomes +1, stringifies "A2".
    assert_eq!(
        try_point_move(&ctx("=", 1, false, at_a1(), None), "ArrowDown", false),
        PointMoveOutcome::Move(PointingStep {
            text: "=A2".to_string(),
            ref_node: RefNode::cell(
                1,
                None,
                1,
                0,
                Absolute {
                    row: false,
                    column: false
                }
            ),
            span: TextRef { start: 1, end: 3 },
        }),
    );
}

#[wasm_bindgen_test]
fn point_move_shift_extends_anchor() {
    // Already pointing at B3 with editing A1: anchor stored as (2,1) delta.
    // Shift+ArrowDown extends trailing only -> RangeKind (2,1)..(3,1) -> "B3:B4".
    assert_eq!(
        try_point_move(
            &ctx("=B3", 3, true, at_b3(), Some(TextRef { start: 1, end: 3 })),
            "ArrowDown",
            true,
        ),
        PointMoveOutcome::Move(PointingStep {
            text: "=B3:B4".to_string(),
            ref_node: RefNode::range(
                1,
                None,
                2,
                1,
                Absolute {
                    row: false,
                    column: false
                },
                3,
                1,
                Absolute {
                    row: false,
                    column: false
                }
            ),
            span: TextRef { start: 1, end: 6 },
        }),
    );
}

/// Reproducer: user editing C1 with `=$A$1+` at cursor=6 presses ArrowDown.
/// Expected: point-mode engages, splices `C2` producing `=$A$1+C2`.
#[allow(clippy::panic)]
#[test]
fn point_move_reproduce_abs_then_operator_then_arrow() {
    use crate::coord::SheetRange;
    let editing = CellAddress {
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
    // delta (2,1) + (0,1) = (2,2) -> stringify C3.
    assert_eq!(
        try_point_move(
            &ctx("=B3", 3, true, at_b3(), Some(TextRef { start: 1, end: 3 })),
            "ArrowRight",
            false,
        ),
        PointMoveOutcome::Move(PointingStep {
            text: "=C3".to_string(),
            ref_node: RefNode::cell(
                1,
                None,
                2,
                2,
                Absolute {
                    row: false,
                    column: false
                }
            ),
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
    // insertion point, but prev_span is seeded from the caret-hit ->
    // the new guard accepts entry. ArrowDown must emit `$A$2`, not
    // `A2` — both absolute flags survive `extend_trailing`.
    let ctx = PointMoveCtx {
        text: "=$A$1",
        cursor: 3,
        already_pointing: false,
        current_ref: RefNode::cell(
            1,
            None,
            1,
            1,
            Absolute {
                row: true,
                column: true,
            },
        ),
        prev_span: Some(TextRef { start: 1, end: 5 }),
        editing: editing_a1(),
    };
    assert_eq!(
        try_point_move(&ctx, "ArrowDown", false),
        PointMoveOutcome::Move(PointingStep {
            text: "=$A$2".to_string(),
            ref_node: RefNode::cell(
                1,
                None,
                2,
                1,
                Absolute {
                    row: true,
                    column: true
                }
            ),
            span: TextRef { start: 1, end: 5 },
        }),
    );
}

#[test]
fn caret_on_cross_sheet_ref_preserves_sheet() {
    // `=Sheet2!B2`, caret inside `B2` (position 9). With editing=A1 the
    // relative deltas for B2 are (+1,+1). ArrowRight moves the column
    // delta to +2 -> absolute C2 on Sheet2. `sheet_name` survives into
    // the new RefNode; `to_localized` re-emits the `Sheet2!` prefix.
    let ctx = PointMoveCtx {
        text: "=Sheet2!B2",
        cursor: 9,
        already_pointing: false,
        current_ref: RefNode::cell(
            2,
            Some("Sheet2".to_string()),
            1,
            1,
            Absolute {
                row: false,
                column: false,
            },
        ),
        prev_span: Some(TextRef { start: 1, end: 10 }),
        editing: editing_a1(),
    };
    assert_eq!(
        try_point_move(&ctx, "ArrowRight", false),
        PointMoveOutcome::Move(PointingStep {
            text: "=Sheet2!C2".to_string(),
            ref_node: RefNode::cell(
                2,
                Some("Sheet2".to_string()),
                1,
                2,
                Absolute {
                    row: false,
                    column: false
                }
            ),
            span: TextRef { start: 1, end: 10 },
        }),
    );
}

#[test]
fn caret_on_relative_ref_stays_relative() {
    // `=A1`, caret between A and 1. Fully relative -> no `$` introduced.
    // This is the control case: absence of flags must not cause
    // `extend_trailing` to invent them.
    let ctx = PointMoveCtx {
        text: "=A1",
        cursor: 2,
        already_pointing: false,
        current_ref: RefNode::cell(
            1,
            None,
            0,
            0,
            Absolute {
                row: false,
                column: false,
            },
        ),
        prev_span: Some(TextRef { start: 1, end: 3 }),
        editing: editing_a1(),
    };
    assert_eq!(
        try_point_move(&ctx, "ArrowDown", false),
        PointMoveOutcome::Move(PointingStep {
            text: "=A2".to_string(),
            ref_node: RefNode::cell(
                1,
                None,
                1,
                0,
                Absolute {
                    row: false,
                    column: false
                }
            ),
            span: TextRef { start: 1, end: 3 },
        }),
    );
}
