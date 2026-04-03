/// Pure helpers for formula point-mode editing.
///
/// These operate on formula strings and cursor positions; they have no side
/// effects and do not touch the model.
use ironcalc_base::expressions::utils::number_to_column;
use wasm_bindgen::JsCast;

// DOM cursor helper

// TODO: can this use active_cell?
/// Return the `selectionEnd` cursor position of the currently focused formula
/// input (cell textarea or formula-bar input). Returns 0 on failure.
pub fn get_formula_cursor() -> usize {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .and_then(|el| {
            el.clone()
                .dyn_into::<web_sys::HtmlTextAreaElement>()
                .ok()
                .and_then(|ta| ta.selection_end().ok().flatten())
                .or_else(|| {
                    el.dyn_into::<web_sys::HtmlInputElement>()
                        .ok()
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
        // NOTE: confirm valid ','
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
/// Includes a sheet prefix (`"Sheet2!B4"`) only when `ref_sheet` differs from
/// `active_sheet`.  `sheet_name` is the name of `ref_sheet`.
pub fn range_ref_str(
    r1: i32,
    c1: i32,
    r2: i32,
    c2: i32,
    ref_sheet: u32,
    active_sheet: u32,
    sheet_name: &str,
) -> String {
    let top_left = cell_ref_str(r1.min(r2), c1.min(c2));
    let bot_right = cell_ref_str(r1.max(r2), c1.max(c2));
    let range = if top_left == bot_right {
        top_left
    } else {
        format!("{top_left}:{bot_right}")
    };
    if ref_sheet != active_sheet {
        format!("{sheet_name}!{range}")
    } else {
        range
    }
}

// In-formula reference splicing

/// Replace or insert a reference string inside a formula.
///
/// * If `prev_span` is `Some((start, end))`, the substring `text[start..end]`
///   is assumed to be the previous reference and is replaced in-place.
/// * Otherwise `ref_str` is inserted at `cursor`.
///
/// Returns `(new_text, new_span_start, new_span_end)` so the caller can store
/// the new span and replace it again on the next arrow keypress or cell click.
pub fn splice_ref(
    text: &str,
    cursor: usize,
    ref_str: &str,
    prev_span: Option<(usize, usize)>,
) -> (String, usize, usize) {
    let (start, end) = prev_span.unwrap_or((cursor, cursor));
    // Guard against out-of-range spans (e.g. after the user typed extra chars).
    let start = start.min(text.len());
    let end = end.min(text.len()).max(start);
    let new_text = format!("{}{}{}", &text[..start], ref_str, &text[end..]);
    let new_end = start + ref_str.len();
    (new_text, start, new_end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    // is_in_reference_mode

    #[wasm_bindgen_test]
    fn ref_mode_empty_string() {
        assert_eq!(is_in_reference_mode("", 0), false);
    }

    #[wasm_bindgen_test]
    fn ref_mode_no_equals_plain_word() {
        assert_eq!(is_in_reference_mode("hello", 5), false);
    }

    #[wasm_bindgen_test]
    fn ref_mode_no_equals_number() {
        assert_eq!(is_in_reference_mode("100", 3), false);
    }

    #[wasm_bindgen_test]
    fn ref_mode_bare_equals() {
        assert_eq!(is_in_reference_mode("=", 1), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_open_paren() {
        assert_eq!(is_in_reference_mode("=SUM(", 5), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_plus() {
        assert_eq!(is_in_reference_mode("=A1+", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_minus() {
        assert_eq!(is_in_reference_mode("=A1-", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_star() {
        assert_eq!(is_in_reference_mode("=A1*", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_slash() {
        assert_eq!(is_in_reference_mode("=A1/", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_comma() {
        assert_eq!(is_in_reference_mode("=A1,", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_ampersand() {
        assert_eq!(is_in_reference_mode("=A1&", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_after_colon() {
        assert_eq!(is_in_reference_mode("=A1:", 4), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_cursor_at_end_of_ref_token() {
        // Cursor sits right after a cell reference - not a valid insertion point.
        assert_eq!(is_in_reference_mode("=A1", 3), false);
    }

    #[wasm_bindgen_test]
    fn ref_mode_cursor_beyond_len_clamped() {
        // cursor > text.len() should clamp to text.len() and still return true.
        assert_eq!(is_in_reference_mode("=SUM(", 100), true);
    }

    #[wasm_bindgen_test]
    fn ref_mode_space_before_operator_trim_end() {
        // trim_end strips trailing whitespace so the last meaningful char is '+'.
        assert_eq!(is_in_reference_mode("=A1 +", 5), true);
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
        assert_eq!(range_ref_str(1, 1, 1, 1, 1, 1, ""), "A1");
    }

    #[wasm_bindgen_test]
    fn range_ref_multi_cell_same_sheet() {
        assert_eq!(range_ref_str(1, 1, 3, 2, 1, 1, ""), "A1:B3");
    }

    #[wasm_bindgen_test]
    fn range_ref_cross_sheet_single_cell() {
        assert_eq!(range_ref_str(1, 1, 1, 1, 2, 1, "Sheet2"), "Sheet2!A1");
    }

    #[wasm_bindgen_test]
    fn range_ref_cross_sheet_range() {
        assert_eq!(range_ref_str(1, 1, 3, 2, 2, 1, "Sheet2"), "Sheet2!A1:B3");
    }

    #[wasm_bindgen_test]
    fn range_ref_reversed_coords_normalize() {
        // r1/c1 and r2/c2 are swapped - min/max normalization must produce A1:B3.
        assert_eq!(range_ref_str(3, 2, 1, 1, 1, 1, ""), "A1:B3");
    }

    // splice_ref

    #[wasm_bindgen_test]
    fn splice_insert_at_cursor_no_prev_span() {
        assert_eq!(
            splice_ref("=SUM(", 5, "A1", None),
            ("=SUM(A1".to_string(), 5, 7)
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_prev_span() {
        assert_eq!(
            splice_ref("=SUM(A1)", 7, "B2", Some((5, 7))),
            ("=SUM(B2)".to_string(), 5, 7)
        );
    }

    #[wasm_bindgen_test]
    fn splice_insert_after_equals() {
        assert_eq!(splice_ref("=", 1, "A1", None), ("=A1".to_string(), 1, 3));
    }

    #[wasm_bindgen_test]
    fn splice_span_out_of_range_clamps() {
        // prev_span (10, 15) is beyond text length 3 - clamps to (3, 3) -> append.
        assert_eq!(
            splice_ref("=A1", 3, "B2", Some((10, 15))),
            ("=A1B2".to_string(), 3, 5)
        );
    }

    #[wasm_bindgen_test]
    fn splice_replace_extends_span_when_ref_is_longer() {
        assert_eq!(
            splice_ref("=A1", 3, "Sheet2!A1:B100", Some((1, 3))),
            ("=Sheet2!A1:B100".to_string(), 1, 15)
        );
    }
}
