/// Pure helpers for formula point-mode editing.
///
/// These operate on formula strings and cursor positions; they have no side
/// effects and do not touch the model.
use crate::coord::{CellArea, RefSpan, SheetArea};
use ironcalc_base::expressions::utils::number_to_column;
use wasm_bindgen::JsCast;

// DOM cursor helper

/// Return the `selectionEnd` cursor position of the currently focused formula
/// input (cell textarea or formula-bar input). Returns 0 on failure.
pub fn get_formula_cursor() -> usize {
    leptos::prelude::document()
        .active_element()
        .and_then(|el| {
            el.dyn_ref::<web_sys::HtmlTextAreaElement>()
                .and_then(|ta| ta.selection_end().ok().flatten())
                .or_else(|| {
                    el.dyn_ref::<web_sys::HtmlInputElement>()
                        .and_then(|inp| inp.selection_end().ok().flatten())
                })
        })
        .map(|n| n as usize)
        .unwrap_or(0)
}

// Reference-mode detection

/// Returns `true` if the cursor is at a position in `text` where inserting a
/// cell reference would be syntactically valid.
///
/// Uses a simple heuristic: the text starts with `'='` AND the last
/// non-whitespace character before `cursor` is an operator or opening paren
/// (or the text is exactly `"="`).
pub fn is_in_reference_mode(text: &str, cursor: usize) -> bool {
    if !text.starts_with('=') {
        return false;
    }
    let before = &text[..cursor.min(text.len())];
    if before == "=" {
        return true;
    }
    matches!(
        before.trim_end().chars().last(),
        // NOTE: confirm valid ',' in other locales ie. German decimals sep is '.'
        Some(',' | '(' | '+' | '-' | '*' | '/' | '<' | '>' | '=' | '&' | ';' | ':')
    )
}

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
pub fn splice_ref(text: &str, span: RefSpan, ref_str: &str) -> (String, RefSpan) {
    // Guard against out-of-range spans (e.g. after the user typed extra chars).
    let start = span.start.min(text.len());
    let end = span.end.min(text.len()).max(start);
    let new_text = format!("{}{}{}", &text[..start], ref_str, &text[end..]);
    let new_end = start + ref_str.len();
    (
        new_text,
        RefSpan {
            start,
            end: new_end,
        },
    )
}

// Point-mode move computation

/// Result of a successful point-mode arrow move.
#[derive(Debug, PartialEq)]
pub struct PointingStep {
    /// Formula text with the new reference spliced in.
    pub text: String,
    /// The new pointed-at cell range (for `DragState::Pointing { range }`).
    pub range: CellArea,
    /// Byte span of the spliced reference in `text` (for `DragState::Pointing { ref_span }`).
    pub span: RefSpan,
}

/// Returns `true` when a keypress should exit point mode.
///
/// Any key that is not an arrow key or a bare modifier should drop the
/// `DragState::Pointing` state so the next arrow press starts a fresh reference.
pub fn should_exit_pointing(key: &str) -> bool {
    !matches!(
        key,
        "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight" | "Shift" | "Control" | "Alt" | "Meta"
    )
}

/// Compute a point-mode arrow move from pure inputs. Returns `None` when:
/// - `key` is not an arrow key, or
/// - `already_pointing` is false AND cursor is not at a valid reference insertion point.
///
/// # Caller responsibilities
///
/// - Check `may_point` (`edit.mode == EditMode::Accept || edit.text_dirty || already_pointing`)
///   before calling — this involves `EditMode` (UI state), not formula text.
/// - Read `cursor` from the DOM via `get_formula_cursor()`.
/// - Resolve `current_range` from `WorkbookState::effective_point_range`.
/// - Resolve `prev_span` from `DragState::Pointing { ref_span }`.
/// - Apply signal writes from the returned `PointMoveResult`.
///
/// # Future work
///
/// A `PointingStep` enum (`ExitPointing | Move(PointMoveResult) | NoAction`) could absorb
/// the "exit pointing on non-arrow key" guard in `workbook.rs` as well, making the component a
/// pure dispatcher. Before extracting, review the `DragState`/`EditMode` signal lifecycle in
/// `state.rs` — signal writes from inside an enum producer may change the reactivity shape.
// TODO(future): PointingStep — see doc comment above
pub fn try_point_move(
    text: &str,
    key: &str,
    is_shift: bool,
    cursor: usize,
    already_pointing: bool,
    current_range: CellArea,
    prev_span: Option<RefSpan>,
    sheet: u32,
) -> Option<PointingStep> {
    // Only arrow keys trigger point-mode movement.
    if !matches!(key, "ArrowDown" | "ArrowUp" | "ArrowLeft" | "ArrowRight") {
        return None;
    }
    // Entry guard: must be already pointing or at a valid reference insertion point.
    if !already_pointing && !is_in_reference_mode(text, cursor) {
        return None;
    }
    // Extend the trailing corner of the range one step in the arrow direction.
    let trailing = current_range.extend_trailing(key);
    // Shift extends the selection (anchor stays); plain arrow moves the whole range.
    let new_range = if is_shift {
        CellArea {
            r1: current_range.r1,
            c1: current_range.c1,
            r2: trailing.r2,
            c2: trailing.c2,
        }
    } else {
        CellArea::from_cell(trailing.r2, trailing.c2)
    };
    let ref_str = range_ref_str(new_range.with_sheet(sheet), sheet, "");
    let (new_text, new_span) = splice_ref(text, prev_span.unwrap_or(RefSpan::at(cursor)), &ref_str);
    Some(PointingStep {
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

    // is_in_reference_mode

    #[wasm_bindgen_test]
    fn ref_mode_empty_string() {
        assert!(!is_in_reference_mode("", 0));
    }

    #[wasm_bindgen_test]
    fn ref_mode_no_equals_plain_word() {
        assert!(is_in_reference_mode("hello", 5));
    }

    #[wasm_bindgen_test]
    fn ref_mode_no_equals_number() {
        assert!(is_in_reference_mode("100", 3));
    }

    #[wasm_bindgen_test]
    fn ref_mode_bare_equals() {
        assert!(is_in_reference_mode("=", 1));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_open_paren() {
        assert!(is_in_reference_mode("=SUM(", 5));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_plus() {
        assert!(is_in_reference_mode("=A1+", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_minus() {
        assert!(is_in_reference_mode("=A1-", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_star() {
        assert!(is_in_reference_mode("=A1*", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_slash() {
        assert!(is_in_reference_mode("=A1/", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_comma() {
        assert!(is_in_reference_mode("=A1,", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_ampersand() {
        assert!(is_in_reference_mode("=A1&", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_colon() {
        assert!(is_in_reference_mode("=A1:", 4));
    }

    #[wasm_bindgen_test]
    fn ref_mode_cursor_at_end_of_ref_token() {
        // Cursor sits right after a cell reference - not a valid insertion point.
        assert!(!is_in_reference_mode("=A1", 3));
    }

    #[wasm_bindgen_test]
    fn ref_mode_cursor_beyond_len_clamped() {
        // cursor > text.len() should clamp to text.len() and still return true.
        assert!(is_in_reference_mode("=SUM(", 100));
    }

    #[wasm_bindgen_test]
    fn ref_mode_space_before_operator_trim_end() {
        // trim_end strips trailing whitespace so the last meaningful char is '+'.
        assert!(is_in_reference_mode("=A1 +", 5));
    }

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
            splice_ref("=SUM(", RefSpan::at(5), "A1"),
            ("=SUM(A1".to_string(), RefSpan { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_prev_span() {
        let rs = RefSpan { start: 5, end: 7 };
        assert_eq!(
            splice_ref("=SUM(A1)", rs, "B2"),
            ("=SUM(B2)".to_string(), RefSpan { start: 5, end: 7 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_insert_after_equals() {
        assert_eq!(
            splice_ref("=", RefSpan::at(1), "A1"),
            ("=A1".to_string(), RefSpan { start: 1, end: 3 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_span_out_of_range_clamps() {
        // prev_span (10, 15) is beyond text length 3 - clamps to (3, 3) -> append.
        let rs = RefSpan { start: 10, end: 15 };
        assert_eq!(
            splice_ref("=A1", rs, "B2"),
            ("=A1B2".to_string(), RefSpan { start: 3, end: 5 })
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_extends_span_when_ref_is_longer() {
        let rs = RefSpan { start: 1, end: 3 };
        assert_eq!(
            splice_ref("=A1", rs, "Sheet2!A1:B100"),
            ("=Sheet2!A1:B100".to_string(), RefSpan { start: 1, end: 15 })
        );
    }

    // try_point_move

    #[wasm_bindgen_test]
    fn point_move_non_arrow_key_returns_none() {
        let range = CellArea {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1,
        };
        assert_eq!(
            try_point_move("=", "Enter", false, 1, false, range, None, 1),
            None
        );
    }

    #[wasm_bindgen_test]
    fn point_move_cursor_after_ref_token_not_pointing_returns_none() {
        // Cursor at end of "=A1" (position 3) — last char is '1', not an operator.
        // is_in_reference_mode returns false here, and already_pointing is false.
        let range = CellArea {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1,
        };
        assert_eq!(
            try_point_move("=A1", "ArrowDown", false, 3, false, range, None, 1),
            None
        );
    }

    #[wasm_bindgen_test]
    fn point_move_already_pointing_bypasses_ref_mode_check() {
        // "=A1" cursor at 3 is not in ref mode normally, but already_pointing=true
        // bypasses the is_in_reference_mode guard.
        let range = CellArea {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1,
        };
        let result = try_point_move(
            "=A1",
            "ArrowDown",
            false,
            3,
            true,
            range,
            Some(RefSpan { start: 1, end: 3 }),
            1,
        );
        assert!(result.is_some());
    }

    #[wasm_bindgen_test]
    fn point_move_bare_equals_arrow_down_enters_a2() {
        // "=" cursor=1: is_in_reference_mode returns true (bare equals).
        // ArrowDown from A1 → new_range=A2, ref="A2", splice inserts at cursor.
        let range = CellArea {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1,
        };
        assert_eq!(
            try_point_move("=", "ArrowDown", false, 1, false, range, None, 1),
            Some(PointingStep {
                text: "=A2".to_string(),
                range: CellArea {
                    r1: 2,
                    c1: 1,
                    r2: 2,
                    c2: 1
                },
                span: RefSpan { start: 1, end: 3 },
            })
        );
    }

    #[wasm_bindgen_test]
    fn point_move_shift_extends_anchor() {
        // Already pointing at B3, ArrowDown+Shift: anchor B3 stays, trailing extends to B4.
        let range = CellArea {
            r1: 3,
            c1: 2,
            r2: 3,
            c2: 2,
        };
        assert_eq!(
            try_point_move(
                "=B3",
                "ArrowDown",
                true,
                3,
                true,
                range,
                Some(RefSpan { start: 1, end: 3 }),
                1
            ),
            Some(PointingStep {
                text: "=B3:B4".to_string(),
                range: CellArea {
                    r1: 3,
                    c1: 2,
                    r2: 4,
                    c2: 2
                },
                span: RefSpan { start: 1, end: 6 },
            })
        );
    }

    #[wasm_bindgen_test]
    fn point_move_plain_arrow_moves_whole_range() {
        // Already pointing at B3, ArrowRight (no shift): whole range moves to C3.
        let range = CellArea {
            r1: 3,
            c1: 2,
            r2: 3,
            c2: 2,
        };
        assert_eq!(
            try_point_move(
                "=B3",
                "ArrowRight",
                false,
                3,
                true,
                range,
                Some(RefSpan { start: 1, end: 3 }),
                1
            ),
            Some(PointingStep {
                text: "=C3".to_string(),
                range: CellArea {
                    r1: 3,
                    c1: 3,
                    r2: 3,
                    c2: 3
                },
                span: RefSpan { start: 1, end: 3 },
            })
        );
    }
}
