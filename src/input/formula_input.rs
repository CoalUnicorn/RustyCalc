/// Pure helpers for formula point-mode editing.
///
/// These operate on formula strings and cursor positions; they have no side
/// effects and do not touch the model.
use crate::coord::{CellArea, PointingStep, SheetArea, SpanRef};
use crate::input::formula_analysis::is_in_reference_mode;
use ironcalc_base::expressions::utils::number_to_column;

// Reference string formatting

/// Format a single cell as an A1-style reference string, e.g. `"B6"`.
pub fn cell_ref_str(row: i32, col: i32) -> String {
    let col_name = number_to_column(col).unwrap_or_default();
    format!("{col_name}{row}")
}

/// Format a cell range as `"B4:C7"` (or `"B6"` when it is a single cell).
///
/// Includes a sheet prefix (`"Sheet2!B4"`) only when `area.sheet` differs from
/// `active_sheet`.  `sheet_name` is the display name of `area.sheet`.
// TODO: this should include R1C1 style
pub fn range_ref_str(area: SheetArea, active_sheet: u32, sheet_name: &str) -> String {
    let norm = area.area.normalized();
    let top_left = cell_ref_str(norm.r1, norm.c1);
    let bot_right = cell_ref_str(norm.r2, norm.c2);
    let range = if top_left == bot_right {
        top_left
    } else {
        format!("{top_left}:{bot_right}")
    };
    if area.sheet != active_sheet {
        format!("{sheet_name}!{range}")
    } else {
        range
    }
}

// In-formula reference splicing

/// Replace or insert a reference string inside a formula.
///
/// `span` marks the region to replace: pass an existing `RefSpan` to overwrite
/// a previous reference, or `RefSpan::at(cursor)` to insert at the cursor position.
///
/// Returns `(new_text, new_span)` so the caller can store the span and replace
/// it again on the next arrow keypress or cell click.
pub fn splice_ref(text: &str, span: SpanRef, ref_str: &str) -> (String, SpanRef) {
    // Guard against out-of-range spans (e.g. after the user typed extra chars).
    let start = span.start.min(text.len());
    let end = span.end.min(text.len()).max(start);
    let new_text = format!("{}{}{}", &text[..start], ref_str, &text[end..]);
    let new_end = start + ref_str.len();
    (
        new_text,
        SpanRef {
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
    pub current_range: CellArea,
    pub prev_span: Option<SpanRef>,
    pub sheet: u32,
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
    if !matches!(key, "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight") {
        return PointMoveOutcome::ExitPointing;
    }
    // Arrow key: enter or extend point mode if cursor is at a valid reference insertion point.
    if !ctx.already_pointing && !is_in_reference_mode(ctx.text, ctx.cursor) {
        return PointMoveOutcome::NoAction;
    }
    let trailing = ctx.current_range.extend_trailing(key);
    // Shift extends the selection (anchor stays); plain arrow moves the whole range.
    let new_range = if is_shift {
        CellArea {
            r1: ctx.current_range.r1,
            c1: ctx.current_range.c1,
            r2: trailing.r2,
            c2: trailing.c2,
        }
    } else {
        CellArea::from_cell(trailing.r2, trailing.c2)
    };
    let ref_str = range_ref_str(new_range.with_sheet(ctx.sheet), ctx.sheet, "");
    let (new_text, new_span) = splice_ref(
        ctx.text,
        ctx.prev_span.unwrap_or(SpanRef::at(ctx.cursor)),
        &ref_str,
    );
    PointMoveOutcome::Move(PointingStep {
        text: new_text,
        range: new_range,
        span: new_span,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::CellArea;
    use wasm_bindgen_test::wasm_bindgen_test;

    // cell_ref_str

    #[wasm_bindgen_test]
    fn cell_ref_col_a_row_1() {
        assert_eq!(cell_ref_str(1, 1), "A1");
    }

    #[wasm_bindgen_test]
    fn cell_ref_col_b_row_6() {
        assert_eq!(cell_ref_str(6, 2), "B6");
    }

    #[wasm_bindgen_test]
    fn cell_ref_col_z_row_10() {
        assert_eq!(cell_ref_str(10, 26), "Z10");
    }

    #[wasm_bindgen_test]
    fn cell_ref_col_aa_row_1() {
        assert_eq!(cell_ref_str(1, 27), "AA1");
    }

    #[wasm_bindgen_test]
    fn cell_ref_col_az_row_100() {
        assert_eq!(cell_ref_str(100, 52), "AZ100");
    }

    // range_ref_str

    #[wasm_bindgen_test]
    fn range_ref_single_cell_same_sheet() {
        assert_eq!(
            range_ref_str(
                CellArea {
                    r1: 1,
                    c1: 1,
                    r2: 1,
                    c2: 1
                }
                .with_sheet(1),
                1,
                ""
            ),
            "A1"
        );
    }

    #[wasm_bindgen_test]
    fn range_ref_multi_cell_same_sheet() {
        assert_eq!(
            range_ref_str(
                CellArea {
                    r1: 1,
                    c1: 1,
                    r2: 3,
                    c2: 2
                }
                .with_sheet(1),
                1,
                ""
            ),
            "A1:B3"
        );
    }

    #[wasm_bindgen_test]
    fn range_ref_cross_sheet_single_cell() {
        assert_eq!(
            range_ref_str(
                CellArea {
                    r1: 1,
                    c1: 1,
                    r2: 1,
                    c2: 1
                }
                .with_sheet(2),
                1,
                "Sheet2"
            ),
            "Sheet2!A1"
        );
    }

    #[wasm_bindgen_test]
    fn range_ref_cross_sheet_range() {
        assert_eq!(
            range_ref_str(
                CellArea {
                    r1: 1,
                    c1: 1,
                    r2: 3,
                    c2: 2
                }
                .with_sheet(2),
                1,
                "Sheet2"
            ),
            "Sheet2!A1:B3"
        );
    }

    #[wasm_bindgen_test]
    fn range_ref_reversed_coords_normalize() {
        // r1/c1 and r2/c2 are swapped - normalized() handles min/max to produce A1:B3.
        assert_eq!(
            range_ref_str(
                CellArea {
                    r1: 3,
                    c1: 2,
                    r2: 1,
                    c2: 1
                }
                .with_sheet(1),
                1,
                ""
            ),
            "A1:B3"
        );
    }

    // splice_ref

    #[wasm_bindgen_test]
    fn splice_insert_at_cursor_no_prev_span() {
        assert_eq!(
            splice_ref("=SUM(", SpanRef::at(5), "A1"),
            ("=SUM(A1".to_string(), SpanRef { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_prev_span() {
        let rs = SpanRef { start: 5, end: 7 };
        assert_eq!(
            splice_ref("=SUM(A1)", rs, "B2"),
            ("=SUM(B2)".to_string(), SpanRef { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_insert_after_equals() {
        assert_eq!(
            splice_ref("=", SpanRef::at(1), "A1"),
            ("=A1".to_string(), SpanRef { start: 1, end: 3 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_span_out_of_range_clamps() {
        // prev_span (10, 15) is beyond text length 3 - clamps to (3, 3) -> append.
        let rs = SpanRef { start: 10, end: 15 };
        assert_eq!(
            splice_ref("=A1", rs, "B2"),
            ("=A1B2".to_string(), SpanRef { start: 3, end: 5 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_extends_span_when_ref_is_longer() {
        let rs = SpanRef { start: 1, end: 3 };
        assert_eq!(
            splice_ref("=A1", rs, "Sheet2!A1:B100"),
            ("=Sheet2!A1:B100".to_string(), SpanRef { start: 1, end: 15 })
        );
    }

    // try_point_move

    fn ctx<'a>(
        text: &'a str,
        cursor: usize,
        already_pointing: bool,
        range: CellArea,
        prev_span: Option<SpanRef>,
    ) -> PointMoveCtx<'a> {
        PointMoveCtx { text, cursor, already_pointing, current_range: range, prev_span, sheet: 1 }
    }

    #[wasm_bindgen_test]
    fn point_move_non_arrow_key_exits_pointing() {
        // Enter is a non-modifier non-arrow key — signals the user is done pointing.
        let range = CellArea { r1: 1, c1: 1, r2: 1, c2: 1 };
        assert_eq!(
            try_point_move(&ctx("=", 1, false, range, None), "Enter", false),
            PointMoveOutcome::ExitPointing,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_modifier_key_is_no_action() {
        // Shift alone must not exit pointing (user is extending a selection).
        let range = CellArea { r1: 1, c1: 1, r2: 1, c2: 1 };
        assert_eq!(
            try_point_move(&ctx("=", 1, true, range, None), "Shift", false),
            PointMoveOutcome::NoAction,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_cursor_after_ref_token_not_pointing_is_no_action() {
        // Cursor at end of "=A1" (position 3) — last char is '1', not an operator.
        // is_in_reference_mode returns false; already_pointing is false → NoAction.
        let range = CellArea { r1: 1, c1: 1, r2: 1, c2: 1 };
        assert_eq!(
            try_point_move(&ctx("=A1", 3, false, range, None), "ArrowDown", false),
            PointMoveOutcome::NoAction,
        );
    }

    #[wasm_bindgen_test]
    fn point_move_already_pointing_bypasses_ref_mode_check() {
        // "=A1" cursor at 3 is not in ref mode normally, but already_pointing=true
        // bypasses the is_in_reference_mode guard → Move.
        let range = CellArea { r1: 1, c1: 1, r2: 1, c2: 1 };
        assert!(matches!(
            try_point_move(
                &ctx("=A1", 3, true, range, Some(SpanRef { start: 1, end: 3 })),
                "ArrowDown",
                false,
            ),
            PointMoveOutcome::Move(_),
        ));
    }

    #[wasm_bindgen_test]
    fn point_move_bare_equals_arrow_down_enters_a2() {
        // "=" cursor=1: is_in_reference_mode returns true (bare equals).
        // ArrowDown from A1 → new_range=A2, ref="A2", splice inserts at cursor.
        let range = CellArea { r1: 1, c1: 1, r2: 1, c2: 1 };
        assert_eq!(
            try_point_move(&ctx("=", 1, false, range, None), "ArrowDown", false),
            PointMoveOutcome::Move(PointingStep {
                text: "=A2".to_string(),
                range: CellArea { r1: 2, c1: 1, r2: 2, c2: 1 },
                span: SpanRef { start: 1, end: 3 },
            }),
        );
    }

    #[wasm_bindgen_test]
    fn point_move_shift_extends_anchor() {
        // Already pointing at B3, ArrowDown+Shift: anchor B3 stays, trailing extends to B4.
        let range = CellArea { r1: 3, c1: 2, r2: 3, c2: 2 };
        assert_eq!(
            try_point_move(
                &ctx("=B3", 3, true, range, Some(SpanRef { start: 1, end: 3 })),
                "ArrowDown",
                true,
            ),
            PointMoveOutcome::Move(PointingStep {
                text: "=B3:B4".to_string(),
                range: CellArea { r1: 3, c1: 2, r2: 4, c2: 2 },
                span: SpanRef { start: 1, end: 6 },
            }),
        );
    }

    #[wasm_bindgen_test]
    fn point_move_plain_arrow_moves_whole_range() {
        // Already pointing at B3, ArrowRight (no shift): whole range moves to C3.
        let range = CellArea { r1: 3, c1: 2, r2: 3, c2: 2 };
        assert_eq!(
            try_point_move(
                &ctx("=B3", 3, true, range, Some(SpanRef { start: 1, end: 3 })),
                "ArrowRight",
                false,
            ),
            PointMoveOutcome::Move(PointingStep {
                text: "=C3".to_string(),
                range: CellArea { r1: 3, c1: 3, r2: 3, c2: 3 },
                span: SpanRef { start: 1, end: 3 },
            }),
        );
    }
}
