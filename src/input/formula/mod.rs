//! Formula tokenization, analysis, editing, and keystroke-sync.
//!
//! Single-module home for the entire formula edit surface:
//! - [`analyze_formula`] / [`FormulaAnalysis`] — tokenization + AST walk
//! - [`FormulaStatus`] — diagnostic state of a formula
//! - [`is_in_reference_mode`] — cursor-context query for ref insertion
//! - [`splice_ref`] / [`splice_dragged_ref`] / [`try_point_move`] — pure
//!   transforms on formula text for point-mode editing
//! - [`sync_edit`] / [`edit_sync::FormulaEditState`] — keystroke-to-state pipeline
//!   shared by cell editor, formula bar, and the named-ranges dialog

pub(crate) mod analysis;
mod edit_sync;
pub(crate) mod input;
mod ref_mode;
mod status;

pub use analysis::{FormulaAnalysis, analyze_formula};
pub use edit_sync::{
    insert_newline_at_caret, read_value_and_cursor, suppress_navigation_defaults, sync_edit,
};
pub use input::{PointMoveCtx, PointMoveOutcome, splice_dragged_ref, splice_ref, try_point_move};
pub use ref_mode::is_in_reference_mode;
pub use status::FormulaStatus;
