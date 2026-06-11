//! Key → `SpreadsheetAction` → model mutation pipeline.
//!
//! - [`classify`] — pure key + modifier lookup table (`KeyMod`, `classify_key`)
//! - [`dispatch`] — `execute()`: routes to per-category execute_* helpers
//!
//! See `docs/adding-actions.md` for how to add or modify actions.

mod classify;
mod dispatch;

pub use super::action::SpreadsheetAction;
pub use classify::{KeyMod, classify_key};
pub use dispatch::execute;
