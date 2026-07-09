//! Diagnostic state of a parsed formula.

use ironcalc_base::expressions::lexer::LexerError;

use crate::coord::{ActiveRef, TextRef};

/// Diagnostic state of a formula — exactly one at a time.
///
/// Precedence is baked in at construction by
/// [`super::analyze_formula`]: `ParseError` -> `LexerError` -> `Unresolved`
/// -> `Valid`. The status bar only surfaces the highest-priority state, so
/// collapsing here keeps the "show in the right order" invariant at the
/// type level rather than on every consumer.
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
    Unresolved {
        invalid_refs: Vec<TextRef>,
        invalid_functions: Vec<TextRef>,
        invalid_names: Vec<TextRef>,
        valid_refs: Vec<ActiveRef>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
}
