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
    lexer::{LexerError, util::get_tokens},
    parser::{Node, new_parser_english},
    token::TokenType,
    types::CellReferenceRC,
};

use crate::coord::{
    Absolute, ActiveRef, CellAddress, DefinedName, FormulaRefKind, RefNode, SheetRange, TextRef,
};

/// Empty slice used by [`FormulaAnalysis::refs`] for variants that carry no overlays.
const NO_REFS: &[ActiveRef] = &[];

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
    /// Spans of cell/range tokens written without a sheet qualifier (e.g. `A1`
    /// vs `Sheet1!A1`). Orthogonal to `status`: a Valid formula can still have
    /// bare refs. Consumers that care about scope-relative resolution (the
    /// Manage Named Ranges dialog under Workbook scope) read these to flag
    /// formulas that would otherwise be ambiguous.
    pub bare_ref_spans: Vec<TextRef>,
}

impl FormulaAnalysis {
    pub fn has_bare_refs(&self) -> bool {
        !self.bare_ref_spans.is_empty()
    }

    /// Returns refs the renderer should paint. Empty for error variants whose
    /// AST was too broken to trust (ParseError, LexerError, NotFormula).
    pub fn refs(&self) -> &[ActiveRef] {
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

    /// Refs whose byte-span contains `cursor` — drives cursor-aware UX
    /// (highlight ref under caret, scope autocomplete, arm point-mode replace).
    ///
    /// `cursor` is a byte offset into the formula string this analysis was
    /// produced from; pair it with `EditingCell.cursor`, kept in lock-step by
    /// `sync_edit`. Returns an iterator so callers pay no allocation on the
    /// keystroke hot path — `.next()` gives the 0-or-1 result, `.collect()`
    /// gives all matches (rare, since lexer tokens don't overlap).
    ///
    /// Boundary is inclusive at both ends: `cursor == span.end` IS "on" the
    /// ref. The caret immediately after a ref is the most natural moment to
    /// want "fix this ref" — the user just typed it — and matches the
    /// post-token boundary pattern used by `is_in_reference_mode`.
    pub fn refs_at_cursor(&self, cursor: usize) -> impl Iterator<Item = &ActiveRef> {
        self.refs()
            .iter()
            .filter(move |r| cursor >= r.span.start && cursor <= r.span.end)
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
    Valid { refs: Vec<ActiveRef> },
    /// Parser rejected the AST — some leaves may be missing downstream.
    ParseError(ParseError),
    /// Lexer rejected a token (e.g. `@` outside a table ref). Carries the
    /// structured error from IronCalc so downstream UIs can surface position.
    LexerError(LexerError),
    /// Parsed cleanly but some references, functions, or names don't resolve.
    /// `valid_refs` is the subset that DID resolve and should still paint.
    // TODO(human): rename `refs` -> `invalid_refs` (and `functions` -> `invalid_functions`,
    // `names` -> `invalid_names`) so the three "bad" fields pair symmetrically with
    // `valid_refs`. Update every destructure site: status_bar.rs, formula_bar.rs,
    // and the tests in this file. The goal: `let Unresolved { invalid_refs, valid_refs, .. }`
    // reads unambiguously without needing the docs.
    Unresolved {
        refs: Vec<TextRef>,
        functions: Vec<TextRef>,
        names: Vec<TextRef>,
        valid_refs: Vec<ActiveRef>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}

/// Leaves that correspond 1:1 with `Reference`/`Range` lexer tokens. Emitted
/// in document order; pairs by position with `ref_range_token_spans`.
///
/// `Resolved` carries the full ironcalc Node identity via `RefNode` — so
/// downstream consumers keep `absolute_row` / `absolute_column` / `sheet_name`
/// available for editing-grade features ("fix this ref", point-mode, circular
/// detection). `RefNode` unifies both cell and range kinds, so one variant
/// covers what used to require two.
#[derive(Debug, PartialEq)]
pub(crate) enum RefLeaf {
    /// Parser resolved a cell or range reference, with full Node identity.
    Resolved(RefNode),
    /// Parser rejected the reference — sheet unknown or badly-formed.
    /// Span from the zipped lexer stream flags the source region.
    Unresolved,
}

/// Leaves that correspond to `Ident` lexer tokens — every parser node born
/// from an identifier emits one, so the zip with `fn_ident_spans` stays in
/// sync even when functions, defined names, and unknowns mix in the same
/// formula. `Known` covers resolved functions and tables (no overlay needed);
/// `DefinedName` carries the formula string so the ident span can be painted
/// over the resolved range; the two `Unknown*` variants surface diagnostics.
#[derive(Debug, PartialEq)]
pub(crate) enum IdentLeaf {
    Known,
    /// Resolved defined name. Carries the definition formula (e.g. `"Sheet1!$B$5"`)
    /// so `analyze_formula` can re-parse it and emit a `FormulaRef` for the ident span.
    DefinedName(String),
    UnknownFunction,
    UnknownName,
}

/// Leaves that consume no span — pure diagnostic state from the parser.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DiagnosticLeaf {
    ParseError { message: String, position: usize },
}

/// Tokenize `formula` and extract cell/range references + validation state.
///
/// Returns an empty [`FormulaAnalysis`] for non-formula text (no leading `=`).
///
/// - `active_cell` — the cell being edited. Its `sheet` drives cross-sheet
///   ref resolution; its `row` / `column` drive `RefNode::area` when projecting
///   each resolved ref to the `SheetArea` cached on `FormulaRef` — ironcalc
///   stores relative coordinates as offsets from the stringify ctx, so the
///   editing cell is required to recover absolute coords.
/// - `sheet_names` — `(sheet_index, display_name)` pairs for cross-sheet ref resolution.
///   Unknown sheet names produce no overlay (the ref is silently skipped).
pub fn analyze_formula(
    formula: &str,
    active_cell: CellAddress,
    sheet_names: &[(u32, String)],
    defined_names: &[DefinedName],
) -> FormulaAnalysis {
    if !formula.starts_with('=') || formula.len() < 2 {
        return FormulaAnalysis::default();
    }

    let tokens = get_tokens(formula);
    let mut validation_error: Option<LexerError> = None;
    let mut ref_range_token_spans: Vec<TextRef> = Vec::new();
    let mut fn_ident_spans: Vec<TextRef> = Vec::new();
    // Bare = lexer saw the ref without a `Sheet!` qualifier. Detected here
    // (purely lexical) rather than after parse, since parse fills `sheet_name`
    // from the resolution context and erases the distinction.
    let mut bare_ref_spans: Vec<TextRef> = Vec::new();
    for t in &tokens {
        let span = TextRef {
            start: t.start as usize,
            end: t.end as usize,
        };
        match &t.token {
            TokenType::Reference { sheet: None, .. } | TokenType::Range { sheet: None, .. } => {
                bare_ref_spans.push(span);
                ref_range_token_spans.push(span);
            }
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
    // consider passing model traits directly as the callers will be using them
    let (sheet_name_list, active_sheet_name) = if sheet_names.is_empty() {
        (vec!["Sheet1".to_string()], "Sheet1".to_string())
    } else {
        let names: Vec<String> = sheet_names.iter().map(|(_, n)| n.clone()).collect();
        let active = sheet_names
            .iter()
            .find(|(i, _)| *i == active_cell.sheet)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| names[0].clone());
        (names, active)
    };
    // TODO(perf): `new_parser_english` allocates vectors + a HashMap on every
    // keystroke. Cost is likely sub-ms for short formulas. Before caching the
    // parser in `WorkbookState` (which adds invalidation work on sheet
    // add/rename/delete), measure with a hyperfine bench over 10/50/200-char
    // inputs and only cache if the cost exceeds ~1ms.
    // Plumb defined names into the parser so `=my_range` resolves to
    // `Node::DefinedNameKind` instead of `Node::NamedVariableKind`. The parser
    // takes the tuple form; we convert once here at the boundary.
    let parser_defined_names = defined_names
        .iter()
        .cloned()
        .map(DefinedName::into_ironcalc)
        .collect::<Vec<_>>();
    let mut parser = new_parser_english(sheet_name_list, parser_defined_names, HashMap::new());
    // Parser context = the editing cell. Nodes encode relative coords as
    // offsets from this ctx; `RefNode::area(&active_cell)` reverses the math
    // to recover absolute 1-based coords for the `sheet_area` projection. Keeping
    // the ctx and the projection base in lockstep is what lets `FormulaRef.ref_node`
    // round-trip through `RefNode::to_localized` without a separate conversion.
    let context = CellReferenceRC {
        sheet: active_sheet_name,
        row: active_cell.row,
        column: active_cell.column,
    };
    let ast = parser.parse(&formula[1..], &context);
    let mut ref_leaves: Vec<RefLeaf> = Vec::new();
    let mut ident_leaves: Vec<IdentLeaf> = Vec::new();
    let mut diag_leaves: Vec<DiagnosticLeaf> = Vec::new();
    ast_leaves(&ast, &mut ref_leaves, &mut ident_leaves, &mut diag_leaves);

    // INVESTIGATE:
    // lexer emits 1 Reference span, parser aborts on trailing `+` before yielding a RefLeaf,
    // leaves.len()==0 vs spans.len()==1). Explain the real invariant: BOTH
    // leaf streams can be shorter than their lexer-span streams when a parse
    // error truncates the AST; the `zip` handles this naturally.
    debug_assert!(ref_leaves.len() <= ref_range_token_spans.len());
    debug_assert!(ident_leaves.len() <= fn_ident_spans.len());

    // Identity: same target -> same color slot, regardless of
    // absolute/relative prefix or lexical sheet qualification.
    let mut color_map: HashMap<SheetRange, usize> = HashMap::new();
    let mut assign_slot = |key: SheetRange| -> usize {
        let next = color_map.len();
        *color_map.entry(key).or_insert(next)
    };

    let mut refs: Vec<ActiveRef> = Vec::new();
    let mut invalid_refs: Vec<TextRef> = Vec::new();
    for (leaf, span) in ref_leaves.iter().zip(ref_range_token_spans.iter().copied()) {
        match leaf {
            RefLeaf::Resolved(ref_node) => {
                // Project to SheetArea once so the renderer hot path is a
                // plain field read. Parser resolved to absolute coords
                // (context (0,0) above), so `active_cell` here is only used
                // for the relative-offset math in `RefNode::area`.
                let sheet_area = ref_node.area(&active_cell);
                refs.push(ActiveRef {
                    ref_node: ref_node.clone(),
                    sheet_area,
                    color_idx: assign_slot(sheet_area),
                    span,
                    kind: FormulaRefKind::Direct,
                });
            }
            RefLeaf::Unresolved => invalid_refs.push(span),
        }
    }

    let mut invalid_functions: Vec<TextRef> = Vec::new();
    let mut invalid_names: Vec<TextRef> = Vec::new();
    for (leaf, span) in ident_leaves.iter().zip(fn_ident_spans.iter().copied()) {
        match leaf {
            IdentLeaf::Known => {}
            IdentLeaf::DefinedName(formula) => {
                let maybe_ref = match parser.parse(formula, &context) {
                    Node::ReferenceKind {
                        sheet_name,
                        sheet_index,
                        absolute_row,
                        absolute_column,
                        row,
                        column,
                    } => Some(RefNode::cell(
                        sheet_index,
                        sheet_name,
                        row,
                        column,
                        Absolute {
                            row: absolute_row,
                            column: absolute_column,
                        },
                    )),
                    Node::RangeKind {
                        sheet_name,
                        sheet_index,
                        absolute_row1,
                        absolute_column1,
                        row1,
                        column1,
                        absolute_row2,
                        absolute_column2,
                        row2,
                        column2,
                    } => Some(RefNode::range(
                        sheet_index,
                        sheet_name,
                        row1,
                        column1,
                        Absolute {
                            row: absolute_row1,
                            column: absolute_column1,
                        },
                        row2,
                        column2,
                        Absolute {
                            row: absolute_row2,
                            column: absolute_column2,
                        },
                    )),
                    _ => None,
                };
                if let Some(ref_node) = maybe_ref {
                    let sheet_area = ref_node.area(&active_cell);
                    refs.push(ActiveRef {
                        ref_node,
                        sheet_area,
                        color_idx: assign_slot(sheet_area),
                        span,
                        kind: FormulaRefKind::DefinedName,
                    });
                }
            }
            IdentLeaf::UnknownFunction => invalid_functions.push(span),
            IdentLeaf::UnknownName => invalid_names.push(span),
        }
    }

    let parse_error = diag_leaves
        .into_iter()
        .map(|d| match d {
            DiagnosticLeaf::ParseError { message, position } => {
                Some(ParseError { message, position })
            }
        })
        .next()
        .unwrap_or_default();

    // Precedence cascade — matches the order the status bar used to reconstruct
    // by hand. Parser errors win because they leave the AST partial; lexer
    // errors beat unresolved names because a bad token taints everything
    // downstream anyway. Overlay refs move into `Valid` / `Unresolved::valid_refs`
    // so broken variants can't carry stale paint data.
    let status = if let Some(e) = parse_error {
        FormulaStatus::ParseError(e)
    } else if let Some(e) = validation_error {
        FormulaStatus::LexerError(e)
    } else if !invalid_refs.is_empty() || !invalid_functions.is_empty() || !invalid_names.is_empty()
    {
        FormulaStatus::Unresolved {
            refs: invalid_refs,
            functions: invalid_functions,
            names: invalid_names,
            valid_refs: refs,
        }
    } else {
        FormulaStatus::Valid { refs }
    };

    FormulaAnalysis {
        status,
        bare_ref_spans,
    }
}

/// Flatten `node` into three pre-order streams — one per consumer.
///
/// Document order matters: `ref_out` is zipped with the lexer's
/// `Reference`/`Range` token spans, and `fn_out` with its `Ident` spans. Any
/// reordering here silently desynchronises colors from formula text. Emit
/// compound markers (`NamedFunctionKind`) before their children so the
/// parent-first invariant lines up with the lexer's left-to-right order.
fn ast_leaves(
    node: &Node,
    ref_out: &mut Vec<RefLeaf>,
    ident_out: &mut Vec<IdentLeaf>,
    diag_out: &mut Vec<DiagnosticLeaf>,
) {
    match node {
        // Resolved cell or range reference. `RefLeaf::Resolved` carries a
        // `RefNode` wrapping the original `Node::ReferenceKind | RangeKind`
        // so `absolute_row` / `absolute_column` / `sheet_name` round-trip to
        // editing-grade consumers (point-mode splice, "fix this ref"). Split
        // across two arms because the two Node variants have disjoint fields.
        Node::ReferenceKind {
            sheet_name,
            sheet_index,
            absolute_row,
            absolute_column,
            row,
            column,
        } => ref_out.push(RefLeaf::Resolved(RefNode::cell(
            *sheet_index,
            sheet_name.clone(),
            *row,
            *column,
            Absolute {
                row: *absolute_row,
                column: *absolute_column,
            },
        ))),
        Node::RangeKind {
            sheet_name,
            sheet_index,
            absolute_row1,
            absolute_column1,
            row1,
            column1,
            absolute_row2,
            absolute_column2,
            row2,
            column2,
        } => ref_out.push(RefLeaf::Resolved(RefNode::range(
            *sheet_index,
            sheet_name.clone(),
            *row1,
            *column1,
            Absolute {
                row: *absolute_row1,
                column: *absolute_column1,
            },
            *row2,
            *column2,
            Absolute {
                row: *absolute_row2,
                column: *absolute_column2,
            },
        ))),
        Node::WrongReferenceKind { .. } | Node::WrongRangeKind { .. } => {
            ref_out.push(RefLeaf::Unresolved)
        }
        Node::FunctionKind { args, .. } => {
            ident_out.push(IdentLeaf::Known);
            for arg in args {
                ast_leaves(arg, ref_out, ident_out, diag_out);
            }
        }
        Node::NamedFunctionKind { args, .. } => {
            ident_out.push(IdentLeaf::UnknownFunction);
            for arg in args {
                ast_leaves(arg, ref_out, ident_out, diag_out);
            }
        }
        Node::OpSumKind { left, right, .. }
        | Node::OpProductKind { left, right, .. }
        | Node::OpPowerKind { left, right }
        | Node::OpRangeKind { left, right }
        | Node::OpConcatenateKind { left, right }
        | Node::CompareKind { left, right, .. } => {
            ast_leaves(left, ref_out, ident_out, diag_out);
            ast_leaves(right, ref_out, ident_out, diag_out);
        }
        Node::UnaryKind { right, .. } => ast_leaves(right, ref_out, ident_out, diag_out),
        Node::ImplicitIntersection { child, .. } => ast_leaves(child, ref_out, ident_out, diag_out),
        Node::SpillRangeOperator { child } => ast_leaves(child, ref_out, ident_out, diag_out),
        Node::LambdaDefKind { body, .. } => ast_leaves(body, ref_out, ident_out, diag_out),
        Node::LambdaCallKind { lambda, args } => {
            ast_leaves(lambda, ref_out, ident_out, diag_out);
            for arg in args {
                ast_leaves(arg, ref_out, ident_out, diag_out);
            }
        }
        Node::ParseErrorKind {
            message, position, ..
        } => diag_out.push(DiagnosticLeaf::ParseError {
            message: message.clone(),
            position: *position,
        }),
        // Identifier-ish nodes that consume an Ident lexer token.
        //
        // These three variants all push ONE leaf so the ident_leaves /
        // fn_ident_spans zip stays in lock-step. The mapping determines
        // whether a bare identifier like `=my_range` is silently accepted
        // (pushed as Known) or surfaced as Unresolved in the status bar
        // (pushed as UnknownName). TableNameKind has no table support yet
        // in RustyCalc, so it rides along as Known for now.
        //
        Node::DefinedNameKind((_, _, formula)) => {
            ident_out.push(IdentLeaf::DefinedName(formula.clone()))
        }
        Node::TableNameKind(_) => ident_out.push(IdentLeaf::Known),
        Node::NamedVariableKind { .. } => ident_out.push(IdentLeaf::UnknownName),

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
#[allow(clippy::panic)]
mod formula_analysis_tests {
    use super::*;
    use crate::coord::CellArea;

    /// Test editing-cell fixture with row=0, column=0. Matches the pre-refactor
    /// parser context so Node-relative coords equal their absolute 1-based form
    /// — every existing assertion on `sheet_area.area` stays valid under the
    /// new signature without arithmetic adjustment.
    fn editing_at(sheet: u32) -> CellAddress {
        CellAddress {
            sheet,
            row: 0,
            column: 0,
        }
    }

    /// Stringify ctx paired with `editing_at` — same-sheet context with empty
    /// sheet name, so `to_localized` emits bare A1-style for same-sheet refs
    /// and `Sheet!A1` only when the Node carries `sheet_name: Some(_)`.
    fn ctx_at(sheet_name: &str) -> CellReferenceRC {
        CellReferenceRC {
            sheet: sheet_name.to_string(),
            row: 0,
            column: 0,
        }
    }

    #[test]
    fn test_single_cell_ref() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
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
        let analysis = analyze_formula("=SUM(B2:C4)", editing_at(0), &[], &[]);
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
        let analysis = analyze_formula("=A1+B2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_non_formula_returns_empty() {
        let analysis = analyze_formula("hello", editing_at(0), &[], &[]);
        assert!(analysis.refs().is_empty());
        assert!(matches!(analysis.status, FormulaStatus::NotFormula));
    }

    #[test]
    fn test_cross_sheet_ref_resolved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(analysis.refs()[0].sheet_area.sheet, 1);
    }

    #[test]
    fn test_unknown_sheet_ref_is_skipped() {
        // A reference to a sheet that doesn't exist in sheet_names should produce
        // no overlay rather than a misleading overlay on the active sheet.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 0);
        assert!(matches!(analysis.status, FormulaStatus::Unresolved { .. }));
    }

    #[test]
    fn test_same_cell_shares_color_slot() {
        // Option A: A1 and A1 collapse to one color slot, regardless of $-prefix.
        let analysis = analyze_formula("=A1+$A$1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_distinct_cells_get_distinct_slots() {
        let analysis = analyze_formula("=A1+B2+A1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 3);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[2].color_idx);
        assert_ne!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_range_and_single_share_when_endpoints_match() {
        let analysis = analyze_formula("=A1+A1:A1", editing_at(0), &[], &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
        assert_eq!(analysis.refs().len(), 2);
        assert_eq!(analysis.refs()[0].color_idx, analysis.refs()[1].color_idx);
    }

    #[test]
    fn test_invalid_function_captured() {
        let analysis = analyze_formula("=FOOBAR(1,2)", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved { functions, .. } = &analysis.status else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(functions.len(), 1);
        let span = functions[0];
        assert_eq!(&"=FOOBAR(1,2)"[span.start..span.end], "FOOBAR");
    }

    #[test]
    fn test_known_function_not_flagged() {
        let analysis = analyze_formula("=SUM(A1:A3)", editing_at(0), &[], &[]);
        assert!(matches!(analysis.status, FormulaStatus::Valid { .. }));
    }

    #[test]
    fn test_wrong_sheet_ref_captured() {
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=Ghost!A1", editing_at(0), &sheets, &[]);
        let FormulaStatus::Unresolved { refs, .. } = &analysis.status else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(refs.len(), 1);
        assert!(analysis.refs().is_empty());
    }

    #[test]
    fn test_known_defined_name_resolves_as_valid() {
        // With `my_range` plumbed in, the parser emits DefinedNameKind and
        // the identifier no longer trips the Unresolved path.
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "A1:A10".into(),
        }];
        let analysis = analyze_formula("=my_range+1", editing_at(0), &[], &defined);
        assert!(
            matches!(analysis.status, FormulaStatus::Valid { .. }),
            "expected Valid, got {:?}",
            analysis.status
        );
    }

    #[test]
    fn test_unknown_name_captured() {
        // No defined names  bare identifier parses as NamedVariableKind and
        // must land in Unresolved.names (NOT Unresolved.functions).
        let analysis = analyze_formula("=my_undefined", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved {
            names, functions, ..
        } = &analysis.status
        else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(names.len(), 1, "unknown name should be captured");
        assert!(
            functions.is_empty(),
            "unknown name must NOT leak into functions"
        );
        let span = names[0];
        assert_eq!(&"=my_undefined"[span.start..span.end], "my_undefined");
    }

    #[test]
    fn test_mixed_unknown_name_and_function() {
        // Both diagnostics surface independently — the renderer can style
        // them differently (squiggle vs italic) without ambiguity.
        let analysis = analyze_formula("=my_undefined + FOOBAR(1)", editing_at(0), &[], &[]);
        let FormulaStatus::Unresolved {
            names, functions, ..
        } = &analysis.status
        else {
            panic!("expected Unresolved, got {:?}", analysis.status);
        };
        assert_eq!(names.len(), 1);
        assert_eq!(functions.len(), 1);
    }

    #[test]
    fn test_validation_error_is_human_readable() {
        // LexerError.message (not Debug format) should be used — no "LexerError {" prefix.
        let analysis = analyze_formula("=@invalid", editing_at(0), &[], &[]);
        if let FormulaStatus::LexerError(ref e) = analysis.status {
            assert!(
                !e.message.contains("LexerError"),
                "validation_error should not contain Rust debug output, got: {}",
                e.message
            );
        }
    }

    // Identity preservation — ref_node must carry `absolute_row` /
    // `absolute_column` / `sheet_name` through analysis. These tests fail
    // until `ast_leaves` pushes the full Node via `RefNode::cell` /
    // `RefNode::range` (the TODO(human) hand-off). Until then `refs()` is
    // empty for resolved refs, so `.refs().len()` is 0 and the `refs()[0]`
    // indexing panics — by design, making the stub's presence impossible
    // to miss in test output.

    #[test]
    fn absolute_flags_preserved() {
        // `=$A$1` — both axes absolute. Round-tripping `ref_node` via
        // `to_localized` emits `$A$1` iff the flags reached RefNode. If
        // ast_leaves dropped them (the pre-refactor bug), stringify -> `A1`.
        let analysis = analyze_formula("=$A$1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("")),
            "$A$1"
        );
    }

    #[test]
    fn mixed_absolute_preserved() {
        // Per-axis flags survive independently.
        let analysis = analyze_formula("=$A1+B$2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 2);
        let ctx = ctx_at("");
        assert_eq!(analysis.refs()[0].ref_node.to_localized(&ctx), "$A1");
        assert_eq!(analysis.refs()[1].ref_node.to_localized(&ctx), "B$2");
    }

    #[test]
    fn cross_sheet_name_preserved() {
        let sheets = vec![(0u32, "Sheet1".to_string()), (1u32, "Sheet2".to_string())];
        let analysis = analyze_formula("=Sheet2!A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        // Stringify ctx on Sheet1 — so a Sheet2! prefix only appears if the
        // Node carries `sheet_name: Some("Sheet2")`.
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("Sheet1")),
            "Sheet2!A1"
        );
    }

    #[test]
    fn same_sheet_name_is_none() {
        // Same-sheet ref must NOT acquire a spurious `Sheet1!` prefix just
        // because sheet_names happens to contain the active sheet's entry.
        let sheets = vec![(0u32, "Sheet1".to_string())];
        let analysis = analyze_formula("=A1", editing_at(0), &sheets, &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert_eq!(
            analysis.refs()[0].ref_node.to_localized(&ctx_at("Sheet1")),
            "A1"
        );
    }

    // refs_at_cursor — byte-span hit-test with inclusive boundaries

    #[test]
    fn cursor_inside_single_ref() {
        // `=A1+1` — A1 at bytes [1, 3). cursor=2 is strictly inside.
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(2).count(), 1);
    }

    #[test]
    fn cursor_at_ref_left_edge() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(1).count(), 1);
    }

    #[test]
    fn cursor_at_ref_right_edge_is_inclusive() {
        // cursor=3 sits just after A1's last byte. Under the inclusive
        // right-edge rule, this IS "on" A1 — matching the just-typed-a-ref
        // UX moment.
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(3).count(), 1);
    }

    #[test]
    fn cursor_between_refs_yields_nothing() {
        // `=A1 + B2` — whitespace between A1 (ends at 3) and B2 (starts at 6).
        // cursor=4 sits on the first space — no ref overlap.
        let analysis = analyze_formula("=A1 + B2", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(4).count(), 0);
    }

    #[test]
    fn cursor_inside_range() {
        // `=SUM(A1:B3)` — the whole `A1:B3` is ONE Range token. cursor=7 is
        // on the `:` — inside the range token's span.
        let analysis = analyze_formula("=SUM(A1:B3)", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(7).count(), 1);
    }

    #[test]
    fn cursor_on_non_formula_yields_nothing() {
        // NotFormula -> refs() is empty; cursor query returns nothing regardless.
        let analysis = analyze_formula("hello", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(2).count(), 0);
    }

    #[test]
    fn cursor_on_parse_error_yields_nothing() {
        // `=A1++` — parser rejects the trailing `+`. ParseError variant makes
        // refs() empty (AST too broken to trust) so cursor query yields none.
        let analysis = analyze_formula("=A1++", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs_at_cursor(1).count(), 0);
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

    // ---- FormulaRefKind tagging ----

    #[test]
    fn direct_ref_kind_is_direct() {
        let analysis = analyze_formula("=A1+1", editing_at(0), &[], &[]);
        assert_eq!(analysis.refs().len(), 1);
        assert!(matches!(analysis.refs()[0].kind, FormulaRefKind::Direct));
    }

    #[test]
    fn defined_name_ref_kind_is_defined_name() {
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "A1:A10".into(),
        }];
        let analysis = analyze_formula("=my_range+1", editing_at(0), &[], &defined);
        assert_eq!(analysis.refs().len(), 1);
        assert!(matches!(
            analysis.refs()[0].kind,
            FormulaRefKind::DefinedName
        ));
    }

    #[test]
    fn mixed_emissions_carry_independent_kinds() {
        // `=A1+my_range` emits one Direct (A1) and one DefinedName (my_range)
        // in document order. The two kinds must route independently — order
        // here mirrors token-stream order in `analyze_formula`.
        let defined = vec![DefinedName {
            name: "my_range".into(),
            scope: None,
            formula: "B1:B10".into(),
        }];
        let analysis = analyze_formula("=A1+my_range", editing_at(0), &[], &defined);
        assert_eq!(analysis.refs().len(), 2);
        assert!(matches!(analysis.refs()[0].kind, FormulaRefKind::Direct));
        assert!(matches!(
            analysis.refs()[1].kind,
            FormulaRefKind::DefinedName
        ));
    }
}
