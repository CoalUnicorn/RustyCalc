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

use std::collections::HashMap;

use ironcalc_base::expressions::{
    lexer::util::get_tokens,
    parser::{new_parser_english, Node},
    token::TokenType,
    types::CellReferenceRC,
};

use crate::coord::{CellAddress, CellArea, FormulaRef, SheetArea, SpanRef};

/// Result of tokenizing a formula for UI purposes.
///
/// Produced by [`analyze_formula`] and stored on `EditingCell` so both
/// the formula bar and the canvas renderer read from the same source.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FormulaAnalysis {
    pub refs: Vec<FormulaRef>,
    pub validation_error: Option<String>,
    /// Parser-level error, distinct from `validation_error` which is lexer-level.
    pub parse_error: Option<ParseError>,
    pub invalid_functions: Vec<SpanRef>,
    pub invalid_refs: Vec<SpanRef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

/// Flat stream of AST leaves in pre-order document order — the ordering
/// invariant the downstream zip with lexer tokens depends on.
#[derive(Debug, PartialEq)]
pub(crate) enum AstLeaf {
    CellAddress { address: CellAddress },
    SheetArea { area: SheetArea },
    WrongReference,
    WrongRange,
    InvalidFunction,
    ParseError { message: String, position: usize },
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
    let mut validation_error: Option<String> = None;
    let mut ref_range_token_spans: Vec<SpanRef> = Vec::new();
    let mut fn_ident_spans: Vec<SpanRef> = Vec::new();
    for t in &tokens {
        let span = SpanRef {
            start: t.start as usize,
            end: t.end as usize,
        };
        match &t.token {
            TokenType::Reference { .. } | TokenType::Range { .. } => {
                ref_range_token_spans.push(span);
            }
            TokenType::Ident(_) => fn_ident_spans.push(span),
            TokenType::Illegal(e) if validation_error.is_none() => {
                validation_error = Some(e.message.clone());
            }
            _ => {}
        }
    }

    // Parser needs a non-empty worksheets list with a matching context sheet,
    // otherwise every bare `A1` resolves to WrongReferenceKind.
    let (sheet_name_list, active_sheet_name) = if sheet_names.is_empty() {
        (vec!["Sheet1".to_string()], "Sheet1".to_string())
    } else {
        let names: Vec<String> = sheet_names.iter().map(|(_, n)| n.clone()).collect();
        let active = sheet_names
            .iter()
            .find(|(i, _)| *i == active_sheet)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| names[0].clone());
        (names, active)
    };
    let mut parser = new_parser_english(sheet_name_list, Vec::new(), HashMap::new());
    // Context (0, 0) cancels the parser's relative-offset math so `ReferenceKind`
    // always carries 1-based absolute coords regardless of the `$` prefix.
    let context = CellReferenceRC {
        sheet: active_sheet_name,
        row: 0,
        column: 0,
    };
    let ast = parser.parse(&formula[1..], &context);
    let mut leaves = Vec::new();
    ast_leaves(&ast, &mut leaves);

    // Option A identity: same target -> same color slot, regardless of
    // absolute/relative prefix or lexical sheet qualification.
    let mut color_map: HashMap<SheetArea, usize> = HashMap::new();
    let mut next_slot = 0usize;
    let mut refs: Vec<FormulaRef> = Vec::new();
    let mut invalid_refs: Vec<SpanRef> = Vec::new();
    let mut invalid_functions: Vec<SpanRef> = Vec::new();
    let mut parse_error: Option<ParseError> = None;
    let mut ref_token_idx = 0usize;
    let mut fn_token_idx = 0usize;
    let mut assign_slot = |key: SheetArea| -> usize {
        *color_map.entry(key).or_insert_with(|| {
            let s = next_slot;
            next_slot += 1;
            s
        })
    };
    for leaf in &leaves {
        match leaf {
            AstLeaf::CellAddress { address } => {
                let span = ref_range_token_spans.get(ref_token_idx).copied();
                ref_token_idx += 1;
                if let Some(span) = span {
                    let slot = assign_slot(address.to_sheet_area());
                    refs.push(FormulaRef {
                        sheet_area: address.to_sheet_area(),
                        color_idx: slot,
                        span,
                    });
                }
            }
            AstLeaf::SheetArea { area } => {
                let span = ref_range_token_spans.get(ref_token_idx).copied();
                ref_token_idx += 1;
                if let Some(span) = span {
                    let slot = assign_slot(*area);
                    refs.push(FormulaRef {
                        sheet_area: *area,
                        color_idx: slot,
                        span,
                    });
                }
            }
            AstLeaf::WrongReference | AstLeaf::WrongRange => {
                if let Some(span) = ref_range_token_spans.get(ref_token_idx).copied() {
                    invalid_refs.push(span);
                }
                ref_token_idx += 1;
            }
            AstLeaf::InvalidFunction => {
                if let Some(span) = fn_ident_spans.get(fn_token_idx).copied() {
                    invalid_functions.push(span);
                }
                fn_token_idx += 1;
            }
            AstLeaf::ParseError { message, position } => {
                if parse_error.is_none() {
                    parse_error = Some(ParseError {
                        message: message.clone(),
                        position: *position,
                    });
                }
            }
        }
    }

    FormulaAnalysis {
        refs,
        validation_error,
        parse_error,
        invalid_functions,
        invalid_refs,
    }
}

/// Flatten `node` into a pre-order stream of semantic leaves. Downstream
/// correlation with lexer tokens relies on this ordering — recurse `left`
/// before `right` in binary ops, and emit compound markers before their
/// children so iterator indices stay aligned.
fn ast_leaves(node: &Node, out: &mut Vec<AstLeaf>) {
    match node {
        Node::ReferenceKind {
            sheet_index,
            row,
            column,
            ..
        } => out.push(AstLeaf::CellAddress {
            address: CellAddress {
                sheet: *sheet_index,
                row: *row,
                column: *column,
            },
        }),
        Node::RangeKind {
            sheet_index,
            row1,
            column1,
            row2,
            column2,
            ..
        } => out.push(AstLeaf::SheetArea {
            area: SheetArea {
                sheet: *sheet_index,
                area: CellArea {
                    r1: *row1,
                    c1: *column1,
                    r2: *row2,
                    c2: *column2,
                },
            },
        }),
        Node::WrongReferenceKind { .. } => out.push(AstLeaf::WrongReference),
        Node::WrongRangeKind { .. } => out.push(AstLeaf::WrongRange),
        Node::FunctionKind { args, .. } => {
            for arg in args {
                ast_leaves(arg, out);
            }
        }
        Node::InvalidFunctionKind { args, .. } => {
            out.push(AstLeaf::InvalidFunction);
            for arg in args {
                ast_leaves(arg, out);
            }
        }
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            ast_leaves(left, out);
            ast_leaves(right, out);
        }
        Node::UnaryKind { right, .. } => ast_leaves(right, out),
        Node::ImplicitIntersection { child, .. } => ast_leaves(child, out),
        Node::ParseErrorKind {
            message, position, ..
        } => out.push(AstLeaf::ParseError {
            message: message.clone(),
            position: *position,
        }),
        Node::BooleanKind(_)
        | Node::NumberKind(_)
        | Node::StringKind(_)
        | Node::ArrayKind(_)
        | Node::DefinedNameKind(_)
        | Node::TableNameKind(_)
        | Node::WrongVariableKind(_)
        | Node::ErrorKind(_)
        | Node::EmptyArgKind => {}
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
    fn test_same_cell_shares_color_slot() {
        // Option A: A1 and A1 collapse to one color slot, regardless of $-prefix.
        let analysis = analyze_formula("=A1+$A$1", 0, &[]);
        assert_eq!(analysis.refs.len(), 2);
        assert_eq!(analysis.refs[0].color_idx, analysis.refs[1].color_idx);
    }

    #[test]
    fn test_distinct_cells_get_distinct_slots() {
        let analysis = analyze_formula("=A1+B2+A1", 0, &[]);
        assert_eq!(analysis.refs.len(), 3);
        assert_eq!(analysis.refs[0].color_idx, analysis.refs[2].color_idx);
        assert_ne!(analysis.refs[0].color_idx, analysis.refs[1].color_idx);
    }

    #[test]
    fn test_range_and_single_share_when_endpoints_match() {
        // A1:A1 and A1 canonicalise to the same key under Option A.
        let analysis = analyze_formula("=A1+A1:A1", 0, &[]);
        assert_eq!(analysis.refs.len(), 2);
        assert_eq!(analysis.refs[0].color_idx, analysis.refs[1].color_idx);
    }

    #[test]
    fn test_invalid_function_captured() {
        let analysis = analyze_formula("=FOOBAR(1,2)", 0, &[]);
        assert_eq!(analysis.invalid_functions.len(), 1);
        let span = analysis.invalid_functions[0];
        assert_eq!(&"=FOOBAR(1,2)"[span.start..span.end], "FOOBAR");
    }

    #[test]
    fn test_known_function_not_flagged() {
        let analysis = analyze_formula("=SUM(A1:A3)", 0, &[]);
        assert!(analysis.invalid_functions.is_empty());
    }

    #[test]
    fn test_wrong_sheet_ref_captured() {
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", 0, &sheets);
        assert_eq!(analysis.invalid_refs.len(), 1);
        assert!(analysis.refs.is_empty());
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
