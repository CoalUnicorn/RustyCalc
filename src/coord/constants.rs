//! Playground-local sheet bounds and layout defaults.
//!
//! These values previously came from `iron-canvas-core`'s legacy
//! `geometry::constants`. The engine's own copies (`ironcalc_base::constants`)
//! are `pub(crate)` and unusable from here, so the playground owns the few it
//! still needs.

/// Maximum row index (Excel/OOXML limit). Cell coordinates are clamped to
/// `1..=LAST_ROW`.
pub const LAST_ROW: i32 = 1_048_576;

/// Maximum column index (Excel/OOXML limit). Cell coordinates are clamped to
/// `1..=LAST_COLUMN`.
pub const LAST_COLUMN: i32 = 16_384;

/// Fallback row height when the model returns `None` (row not explicitly sized).
pub const DEFAULT_ROW_HEIGHT: f64 = 21.0;

/// Fallback column width when the model returns `None` (column not explicitly sized).
pub const DEFAULT_COL_WIDTH: f64 = 64.0;
