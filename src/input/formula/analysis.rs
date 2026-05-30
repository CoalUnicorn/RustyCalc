//! Formula tokenization, AST walk, and overlay extraction.
//!
//! Parses formula text via ironcalc's lexer/parser and produces
//! [`FormulaAnalysis`]: a list of colored [`ActiveRef`] overlays (one per
//! cell/range token) plus the highest-priority validation error.
//!
//! Color assignment is index-based — the renderer resolves `color_idx` to an
//! actual color string via `theme::FORMULA_REF_COLORS`, keeping presentation
//! out of this layer.
//!
//! # Named ranges
//! `Ident` tokens may represent named ranges — resolved here via parser's
//! `defined_names` and re-emitted as `ActiveRef` with `FormulaRefKind::DefinedName`.

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

use super::status::{FormulaStatus, ParseError};

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
            invalid_refs,
            invalid_functions,
            invalid_names,
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
