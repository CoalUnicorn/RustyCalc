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
    lexer::{util::get_tokens, LexerError},
    parser::{new_parser_english, Node},
    token::TokenType,
    types::CellReferenceRC,
};

use crate::coord::{CellAddress, CellArea, FormulaRef, SheetArea, SpanRef};

/// Empty slice used by [`FormulaAnalysis::refs`] for variants that carry no overlays.
const NO_REFS: &[FormulaRef] = &[];

/// Result of tokenizing a formula for UI purposes.
///
/// Produced by [`analyze_formula`] and stored on `EditingCell` so both
/// the formula bar and the canvas renderer read from the same source.
///
/// Overlay refs live inside [`FormulaStatus::Valid`] / `Unresolved::valid_refs` —
/// the wrapper exposes [`FormulaAnalysis::refs`] so consumers that only want
/// the paintable refs don't have to match on every variant.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FormulaAnalysis {
    pub status: FormulaStatus,
}

impl FormulaAnalysis {
    /// Returns refs the renderer should paint. Empty for error variants whose
    /// AST was too broken to trust (ParseError, LexerError, NotFormula).
    pub fn refs(&self) -> &[FormulaRef] {
        match &self.status {
            FormulaStatus::Valid { refs } => refs,
            FormulaStatus::Unresolved { valid_refs, .. } => valid_refs,
            FormulaStatus::NotFormula
            | FormulaStatus::ParseError(_)
            | FormulaStatus::LexerError(_) => NO_REFS,
        }
    }

    pub fn has_any_error(&self) -> bool {
        !matches!(
            self.status,
            FormulaStatus::Valid { .. } | FormulaStatus::NotFormula
        )
    }
}

/// Diagnostic state of a formula — exactly one at a time.
///
/// Precedence is baked in at construction by [`analyze_formula`]:
/// `ParseError` > `LexerError` > `Unresolved` > `Valid`. The status bar only
/// surfaces the highest-priority state, so collapsing here keeps the "show
/// in the right order" invariant at the type level rather than on every
/// consumer.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum FormulaStatus {
    /// Text does not start with `=`. The overlay path still runs harmlessly.
    #[default]
    NotFormula,
    /// AST clean, every name resolved. `refs` carries the per-token overlays.
    Valid { refs: Vec<FormulaRef> },
    /// Parser rejected the AST — some leaves may be missing downstream.
    ParseError(ParseError),
    /// Lexer rejected a token (e.g. `@` outside a table ref). Carries the
    /// structured error from IronCalc so downstream UIs can surface position.
    LexerError(LexerError),
    /// Parsed cleanly but some names/functions don't resolve.
    /// `valid_refs` is the subset that DID resolve and should still paint.
    Unresolved {
        refs: Vec<SpanRef>,
        functions: Vec<SpanRef>,
        valid_refs: Vec<FormulaRef>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

/// Leaves that correspond 1:1 with `Reference`/`Range` lexer tokens. Emitted
/// in document order; pairs by position with `ref_range_token_spans`.
#[derive(Debug, PartialEq)]
pub(crate) enum RefLeaf {
    /// Parser resolved a bare cell reference (e.g. `A1`, `$A$1`, `Sheet2!A1`).
    Resolved(CellAddress),
    /// Parser resolved a range reference (e.g. `A1:B3`).
    ResolvedRange(SheetArea),
    /// Parser rejected the reference — sheet unknown or badly-formed.
    /// Span from the zipped lexer stream flags the source region.
    Unresolved,
}

/// Leaves that correspond to `Ident` lexer tokens — every parser node born
/// from an identifier emits one, so the zip with `fn_ident_spans` stays in
/// sync even when multiple functions (valid and invalid) mix in the same
/// formula. Only `Unknown` triggers the unresolved-function diagnostic.
#[derive(Debug, PartialEq)]
pub(crate) enum FnLeaf {
    Known,
    Unknown,
}

/// Leaves that consume no span — pure diagnostic state from the parser.
#[derive(Debug, PartialEq)]
pub(crate) enum DiagnosticLeaf {
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
    let mut validation_error: Option<LexerError> = None;
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
                validation_error = Some(e.clone());
            }
            _ => {}
        }
    }

    // Parser needs a non-empty worksheets list with a matching context sheet,
    // otherwise every bare `A1` resolves to WrongReferenceKind.

    // NOTE: this can be better sheet_names: &[(u32, String)] as argument can be cleaner
    // consider FrontendModel as the callers will be using it
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
    // TODO(perf): `new_parser_english` allocates vectors + a HashMap on every
    // keystroke. Cost is likely sub-ms for short formulas. Before caching the
    // parser in `WorkbookState` (which adds invalidation work on sheet
    // add/rename/delete), measure with a hyperfine bench over 10/50/200-char
    // inputs and only cache if the cost exceeds ~1ms.
    let mut parser = new_parser_english(sheet_name_list, Vec::new(), HashMap::new());
    // Context (0, 0) cancels the parser's relative-offset math so `ReferenceKind`
    // always carries 1-based absolute coords regardless of the `$` prefix.
    let context = CellReferenceRC {
        sheet: active_sheet_name,
        row: 0,
        column: 0,
    };
    let ast = parser.parse(&formula[1..], &context);
    let mut ref_leaves: Vec<RefLeaf> = Vec::new();
    let mut fn_leaves: Vec<FnLeaf> = Vec::new();
    let mut diag_leaves: Vec<DiagnosticLeaf> = Vec::new();
    ast_leaves(&ast, &mut ref_leaves, &mut fn_leaves, &mut diag_leaves);

    // Three independent correlation passes — each zip pairs one leaf stream
    // with the lexer span stream it's promised to align with. A future
    // `RefLeaf` variant can't desync because the zip is over the same
    // iterator. No manual index tracking needed.
    //
    // Refs align strictly: every Reference/Range lexer token produces exactly
    // one parser Reference/Range/Wrong* node. Functions align loosely: a
    // parse error can abort before emitting a node for a trailing Ident
    // (e.g. `=@invalid`), so fn_leaves may be shorter than fn_ident_spans.
    // The `zip` naturally truncates — we just skip reporting the orphan.
    debug_assert_eq!(ref_leaves.len(), ref_range_token_spans.len());
    debug_assert!(fn_leaves.len() <= fn_ident_spans.len());

    // Identity: same target -> same color slot, regardless of
    // absolute/relative prefix or lexical sheet qualification.
    let mut color_map: HashMap<SheetArea, usize> = HashMap::new();
    let mut next_slot = 0usize;
    let mut assign_slot = |key: SheetArea| -> usize {
        *color_map.entry(key).or_insert_with(|| {
            let s = next_slot;
            next_slot += 1;
            s
        })
    };

    let mut refs: Vec<FormulaRef> = Vec::new();
    let mut invalid_refs: Vec<SpanRef> = Vec::new();
    for (leaf, span) in ref_leaves.iter().zip(ref_range_token_spans.iter().copied()) {
        match leaf {
            RefLeaf::Resolved(address) => {
                let sheet_area = address.to_sheet_area();
                refs.push(FormulaRef {
                    sheet_area,
                    color_idx: assign_slot(sheet_area),
                    span,
                });
            }
            RefLeaf::ResolvedRange(area) => {
                refs.push(FormulaRef {
                    sheet_area: *area,
                    color_idx: assign_slot(*area),
                    span,
                });
            }
            RefLeaf::Unresolved => invalid_refs.push(span),
        }
    }

    let mut invalid_functions: Vec<SpanRef> = Vec::new();
    for (leaf, span) in fn_leaves.iter().zip(fn_ident_spans.iter().copied()) {
        match leaf {
            FnLeaf::Known => {}
            FnLeaf::Unknown => invalid_functions.push(span),
        }
    }

    let parse_error = diag_leaves.into_iter().find_map(|d| match d {
        DiagnosticLeaf::ParseError { message, position } => Some(ParseError { message, position }),
    });

    // Precedence cascade — matches the order the status bar used to reconstruct
    // by hand. Parser errors win because they leave the AST partial; lexer
    // errors beat unresolved names because a bad token taints everything
    // downstream anyway. Overlay refs move into `Valid` / `Unresolved::valid_refs`
    // so broken variants can't carry stale paint data.
    let status = if let Some(e) = parse_error {
        FormulaStatus::ParseError(e)
    } else if let Some(e) = validation_error {
        FormulaStatus::LexerError(e)
    } else if !invalid_refs.is_empty() || !invalid_functions.is_empty() {
        FormulaStatus::Unresolved {
            refs: invalid_refs,
            functions: invalid_functions,
            valid_refs: refs,
        }
    } else {
        FormulaStatus::Valid { refs }
    };

    FormulaAnalysis { status }
}

/// Flatten `node` into three pre-order streams — one per consumer.
///
/// Document order matters: `ref_out` is zipped with the lexer's
/// `Reference`/`Range` token spans, and `fn_out` with its `Ident` spans. Any
/// reordering here silently desynchronises colors from formula text. Emit
/// compound markers (`InvalidFunctionKind`) before their children so the
/// parent-first invariant lines up with the lexer's left-to-right order.
fn ast_leaves(
    node: &Node,
    ref_out: &mut Vec<RefLeaf>,
    fn_out: &mut Vec<FnLeaf>,
    diag_out: &mut Vec<DiagnosticLeaf>,
) {
    match node {
        Node::ReferenceKind {
            sheet_index,
            row,
            column,
            ..
        } => ref_out.push(RefLeaf::Resolved(CellAddress {
            sheet: *sheet_index,
            row: *row,
            column: *column,
        })),
        Node::RangeKind {
            sheet_index,
            row1,
            column1,
            row2,
            column2,
            ..
        } => ref_out.push(RefLeaf::ResolvedRange(SheetArea {
            sheet: *sheet_index,
            area: CellArea {
                r1: *row1,
                c1: *column1,
                r2: *row2,
                c2: *column2,
            },
        })),
        Node::WrongReferenceKind { .. } | Node::WrongRangeKind { .. } => {
            ref_out.push(RefLeaf::Unresolved)
        }
        Node::FunctionKind { args, .. } => {
            fn_out.push(FnLeaf::Known);
            for arg in args {
                ast_leaves(arg, ref_out, fn_out, diag_out);
            }
        }
        Node::InvalidFunctionKind { args, .. } => {
            fn_out.push(FnLeaf::Unknown);
            for arg in args {
                ast_leaves(arg, ref_out, fn_out, diag_out);
            }
        }
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            ast_leaves(left, ref_out, fn_out, diag_out);
            ast_leaves(right, ref_out, fn_out, diag_out);
        }
        Node::UnaryKind { right, .. } => ast_leaves(right, ref_out, fn_out, diag_out),
        Node::ImplicitIntersection { child, .. } => ast_leaves(child, ref_out, fn_out, diag_out),
        Node::ParseErrorKind {
            message, position, ..
        } => diag_out.push(DiagnosticLeaf::ParseError {
            message: message.clone(),
            position: *position,
        }),
        // Identifier-ish nodes that consume an Ident lexer token. Emit
        // `Known` so the fn_leaves / fn_ident_spans zip stays in lock-step;
        // none of these are flagged as unresolved today (matching the pre-
        // split correlation loop).
        Node::DefinedNameKind(_) | Node::TableNameKind(_) | Node::WrongVariableKind(_) => {
            fn_out.push(FnLeaf::Known)
        }
        Node::BooleanKind(_)
        | Node::NumberKind(_)
        | Node::StringKind(_)
        | Node::ArrayKind(_)
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
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].sheet_area.area,
            CellArea {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1
            }
        );
        assert_eq!(analysis.refs()[0].sheet_area.sheet, 0);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
    }

    #[test]
    fn test_range_ref() {
        let analysis = analyze_formula("=SUM(B2:C4)", 0, &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].sheet_area.area,
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
        assert_eq!(analysis.refs().len(), 2);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_non_formula_returns_empty() {
        let analysis = analyze_formula("hello", 0, &[]);
        assert!(analysis.refs().is_empty());
        assert!(matches!(analysis.status, FormulaStatus::NotFormula));
    }

    #[test]
    fn test_cross_sheet_ref_resolved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", 0, &sheets);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(analysis.refs()[0].sheet_area.sheet, 1);
    }

    #[test]
    fn test_unknown_sheet_ref_is_skipped() {
        // A reference to a sheet that doesn't exist in sheet_names should produce
        // no overlay rather than a misleading overlay on the active sheet.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", 0, &sheets);
        assert_eq!(analysis.refs().len(), 0);
        assert!(matches!(analysis.status, FormulaStatus::Unresolved { .. }));
    }

    #[test]
    fn test_same_cell_shares_color_slot() {
        // Option A: A1 and A1 collapse to one color slot, regardless of $-prefix.
        let analysis = analyze_formula("=A1+$A$1", 0, &[]);
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_distinct_cells_get_distinct_slots() {
        let analysis = analyze_formula("=A1+B2+A1", 0, &[]);
        assert_eq!(analysis.refs().len(), 3);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[2].color_idx);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_range_and_single_share_when_endpoints_match() {
        let analysis = analyze_formula("=A1+A1:A1", 0, &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_invalid_function_captured() {
        let analysis = analyze_formula("=FOOBAR(1,2)", 0, &[]);
        let FormulaStatus::Unresolved { functions, .. } = &analysis.status else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(functions.len(), 1);
        let span = functions[0];
        assert_eq!(&"=FOOBAR(1,2)"[span.start..span.end], "FOOBAR");
    }

    #[test]
    fn test_known_function_not_flagged() {
        let analysis = analyze_formula("=SUM(A1:A3)", 0, &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
    }

    #[test]
    fn test_wrong_sheet_ref_captured() {
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", 0, &sheets);
        let FormulaStatus::Unresolved { refs, .. } = &analysis.status else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(refs.len(), 1);
        assert!(analysis.refs().is_empty());
    }

    #[test]
    fn test_validation_error_is_human_readable() {
        // LexerError.message (not Debug format) should be used — no "LexerError {" prefix.
        let analysis = analyze_formula("=@invalid", 0, &[]);
        if let FormulaStatus::LexerError(ref e) = analysis.status {
            assert!(
                !e.message.contains("LexerError"),
                "validation_error should not contain Rust debug output, got: {}",
                e.message
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
