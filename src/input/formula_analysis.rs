//! Formula tokenization and overlay analysis.
//!
//! Parses formula text via ironcalc's lexer and produces [`FormulaAnalysis`]:
//! a list of colored [`FormulaRef`] overlays (one per cell/range token) plus
//! an optional validation error for the first illegal token found.
//!
//! This is the only module that imports ironcalc token types. Color assignment
//! is index-based — the renderer resolves `color_idx` to an actual color string
//! via `theme::FORMULA_REF_COLORS`, keeping presentation out of this layer.
//!
//! # Named ranges
//! `Ident` tokens may represent named ranges — not yet resolved.
//! TODO(named_ranges): resolve Ident tokens via `model.get_defined_name_list()`
//! when the name manager is implemented.

use ironcalc_base::expressions::{lexer::util::get_tokens, token::TokenType};

use crate::coord::{CellArea, FormulaRef, SheetArea, SpanRef};

/// Result of tokenizing a formula for UI purposes.
///
/// Produced by [`analyze_formula`] and stored on `EditingCell` so both
/// the formula bar and the canvas renderer read from the same source.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FormulaAnalysis {
    /// Colored overlays for the canvas, one per distinct cell/range token.
    pub refs: Vec<FormulaRef>,
    /// First illegal token error, if any. `None` means syntactically valid
    /// (or the text is not a formula).
    pub validation_error: Option<String>,
}

/// Tokenize `formula` and extract cell/range references + validation state.
///
/// Returns an empty [`FormulaAnalysis`] for non-formula text (no leading `=`).
///
/// - `active_sheet` — 0-based index of the sheet being edited.
/// - `sheet_names` — `(sheet_index, display_name)` pairs for cross-sheet ref resolution.
///   Unknown sheet names produce no overlay (the ref is silently skipped).
pub fn analyze_formula(
    formula: &str,
    active_sheet: u32,
    sheet_names: &[(u32, String)],
) -> FormulaAnalysis {
    if !formula.starts_with('=') || formula.len() < 2 {
        return FormulaAnalysis::default();
    }

    let tokens = get_tokens(formula);
    let mut refs = Vec::new();
    let mut color_idx = 0usize;
    let mut validation_error: Option<String> = None;

    for token in &tokens {
        match &token.token {
            TokenType::Reference {
                sheet, row, column, ..
            } => {
                let Some(sheet_idx) =
                    resolve_sheet_name(sheet.as_deref(), active_sheet, sheet_names)
                else {
                    continue;
                };
                refs.push(FormulaRef {
                    sheet_area: SheetArea::from_cell(sheet_idx, *row, *column),
                    color_idx,
                    span: SpanRef {
                        start: token.start as usize,
                        end: token.end as usize,
                    },
                });
                color_idx += 1;
            }
            TokenType::Range { sheet, left, right } => {
                let Some(sheet_idx) =
                    resolve_sheet_name(sheet.as_deref(), active_sheet, sheet_names)
                else {
                    continue;
                };
                refs.push(FormulaRef {
                    sheet_area: SheetArea {
                        sheet: sheet_idx,
                        area: CellArea {
                            r1: left.row,
                            c1: left.column,
                            r2: right.row,
                            c2: right.column,
                        },
                    },
                    color_idx,
                    span: SpanRef {
                        start: token.start as usize,
                        end: token.end as usize,
                    },
                });
                color_idx += 1;
            }
            TokenType::Illegal(e) => {
                if validation_error.is_none() {
                    validation_error = Some(e.message.clone());
                }
            }
            // TODO(named_ranges): Ident tokens may be named ranges
            _ => {}
        }
    }

    FormulaAnalysis {
        refs,
        validation_error,
    }
}

/// Resolve an optional cross-sheet name to a sheet index.
///
/// Returns `Some(active_sheet)` for same-sheet refs (no sheet prefix).
/// Returns `None` for unrecognised sheet names — callers skip those refs.
fn resolve_sheet_name(
    name: Option<&str>,
    active_sheet: u32,
    sheet_names: &[(u32, String)],
) -> Option<u32> {
    match name {
        None => Some(active_sheet),
        Some(n) => sheet_names
            .iter()
            .find(|(_, s)| s.eq_ignore_ascii_case(n))
            .map(|(idx, _)| *idx),
    }
}

/// Returns `true` if `cursor` is at a position in `text` where inserting a
/// cell reference would be syntactically valid.
///
/// Tokenizes the formula up to `cursor` via the ironcalc lexer, skips any
/// trailing [`TokenType::Illegal`] tokens (partial input mid-typing, e.g. the
/// `"B"` in `"=A1+B"`), then checks whether the last meaningful token is an
/// operator or opening delimiter that allows a reference to follow.
pub fn is_in_reference_mode(text: &str, cursor: usize) -> bool {
    if !text.starts_with('=') {
        return false;
    }
    let end = cursor.min(text.len());
    if end <= 1 {
        return true;
    }
    let tokens = get_tokens(&text[..end]);
    // Trailing Illegal tokens represent partial input the user is still typing.
    // Skip them to find the last syntactically complete token before the cursor.
    let last = tokens
        .iter()
        .rev()
        .find(|t| !matches!(t.token, TokenType::Illegal(_) | TokenType::EOF));
    match last {
        None => true,
        Some(t) => matches!(
            t.token,
            TokenType::Addition(_)
                | TokenType::Product(_)
                | TokenType::Compare(_)
                | TokenType::Power
                | TokenType::LeftParenthesis
                | TokenType::Semicolon
                | TokenType::Comma
                | TokenType::And
                | TokenType::Colon
        ),
    }
}

#[cfg(test)]
mod formula_analysis_tests {
    use super::*;
    use crate::coord::CellArea;

    #[test]
    fn test_single_cell_ref() {
        let analysis = analyze_formula("=A1+1", 0, &[]);
        assert_eq!(analysis.refs.len(), 1);
        assert_eq!(
            analysis.refs[0].sheet_area.area,
            CellArea {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1
            }
        );
        assert_eq!(analysis.refs[0].sheet_area.sheet, 0);
        assert!(analysis.validation_error.is_none());
    }

    #[test]
    fn test_range_ref() {
        let analysis = analyze_formula("=SUM(B2:C4)", 0, &[]);
        assert_eq!(analysis.refs.len(), 1);
        assert_eq!(
            analysis.refs[0].sheet_area.area,
            CellArea {
                r1: 2,
                c1: 2,
                r2: 4,
                c2: 3
            }
        );
    }

    #[test]
    fn test_multiple_refs_get_different_color_indices() {
        let analysis = analyze_formula("=A1+B2", 0, &[]);
        assert_eq!(analysis.refs.len(), 2);
        assert_ne!(analysis.refs[0].color_idx, analysis.refs[1].color_idx);
    }

    #[test]
    fn test_non_formula_returns_empty() {
        let analysis = analyze_formula("hello", 0, &[]);
        assert!(analysis.refs.is_empty());
        assert!(analysis.validation_error.is_none());
    }

    #[test]
    fn test_cross_sheet_ref_resolved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", 0, &sheets);
        assert_eq!(analysis.refs.len(), 1);
        assert_eq!(analysis.refs[0].sheet_area.sheet, 1);
    }

    #[test]
    fn test_unknown_sheet_ref_is_skipped() {
        // A reference to a sheet that doesn't exist in sheet_names should produce
        // no overlay rather than a misleading overlay on the active sheet.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", 0, &sheets);
        assert_eq!(analysis.refs.len(), 0);
        assert!(analysis.validation_error.is_none());
    }

    #[test]
    fn test_validation_error_is_human_readable() {
        // LexerError.message (not Debug format) should be used — no "LexerError {" prefix.
        let analysis = analyze_formula("=@invalid", 0, &[]);
        if let Some(ref msg) = analysis.validation_error {
            assert!(
                !msg.contains("LexerError"),
                "validation_error should not contain Rust debug output, got: {msg}"
            );
        }
    }

    // is_in_reference_mode

    #[test]
    fn ref_mode_empty_string() {
        assert!(!is_in_reference_mode("", 0));
    }

    #[test]
    fn ref_mode_bare_equals() {
        assert!(is_in_reference_mode("=", 1));
    }

    #[test]
    fn ref_mode_after_open_paren() {
        assert!(is_in_reference_mode("=SUM(", 5));
    }

    #[test]
    fn ref_mode_after_plus() {
        assert!(is_in_reference_mode("=A1+", 4));
    }

    #[test]
    fn ref_mode_after_minus() {
        assert!(is_in_reference_mode("=A1-", 4));
    }

    #[test]
    fn ref_mode_after_star() {
        assert!(is_in_reference_mode("=A1*", 4));
    }

    #[test]
    fn ref_mode_after_slash() {
        assert!(is_in_reference_mode("=A1/", 4));
    }

    #[test]
    fn ref_mode_after_comma() {
        assert!(is_in_reference_mode("=A1,", 4));
    }

    #[test]
    fn ref_mode_after_ampersand() {
        assert!(is_in_reference_mode("=A1&", 4));
    }

    #[test]
    fn ref_mode_after_colon() {
        assert!(is_in_reference_mode("=A1:", 4));
    }

    #[test]
    fn ref_mode_cursor_at_end_of_ref_token() {
        assert!(!is_in_reference_mode("=A1", 3));
    }

    #[test]
    fn ref_mode_cursor_beyond_len_clamped() {
        assert!(is_in_reference_mode("=SUM(", 100));
    }

    #[test]
    fn ref_mode_space_before_operator() {
        assert!(is_in_reference_mode("=A1 +", 5));
    }

    #[test]
    fn ref_mode_after_string_literal() {
        // Cursor after closing quote of a string — should NOT enter ref mode.
        assert!(!is_in_reference_mode("=\"hello\"", 8));
    }

    #[test]
    fn ref_mode_after_power() {
        assert!(is_in_reference_mode("=A1^", 4));
    }

    #[test]
    fn ref_mode_partial_ident_after_operator() {
        // "B" is lexed as Ident (a valid identifier), not Illegal.
        // Reference mode is false mid-identifier — user must clear it first (e.g. backspace).
        assert!(!is_in_reference_mode("=A1+B", 5));
    }
}
