//! Shared coordinate primitives for cell ranges and addresses.
//!
//! All indices are 1-based, matching ironcalc conventions.
//! ironcalc boundary types (`Area`, `ClipboardTuple`, `SelectedView.range`)
//! are converted at the edges via `to_ironcalc_area()`, `as_tuple()`, and
//! `From<[i32; 4]>` — they never leak past the model trait boundary.

mod constants;
mod convert;
mod types;

pub use constants::*;
pub use types::*;
