//! Key → `SpreadsheetAction` → model mutation pipeline.
//!
//! - [`classify`] — pure key + modifier lookup table (`KeyMod`, `classify_key`)
//! - [`dispatch`] — `execute()`: routes to per-category execute_* helpers
//!
//! To add an action: add a variant to the relevant category enum
//! (`NavAction` / `EditAction` / `FormatAction` / `StructAction`), handle it in
//! that category's `execute_*` (`nav.rs` / `edit.rs` / `format.rs` /
//! `structure.rs`), then map the key to it in [`classify`]. Toolbar buttons
//! reuse the same `SpreadsheetAction` via convenience constructors, bypassing
//! key classification.

mod classify;
mod dispatch;

pub use super::action::SpreadsheetAction;
pub use classify::{KeyMod, classify_key};
pub use dispatch::execute;
