//! Public address and UI types for the canvas surface.
//!
//! - [`coord`] — cell-space addressing: `RCRange`, `SheetArea`,
//!   `FormulaRef`, `AutofillTarget`.
//! - [`ui`] — pointer-resolution outcomes: `HitTest`, `ResizeTarget`.
//!
//! Renderer-ready `*Paint` snapshots (`CellPaint`, `BorderPaint`,
//! `TextPaint`) live alongside the paint code in `crate::renderer::cell`,
//! not here.

pub mod coord;
pub mod ui;
