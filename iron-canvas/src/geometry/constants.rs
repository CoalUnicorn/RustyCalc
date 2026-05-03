// Layout constants
pub(crate) const SELECTION_BORDER_WIDTH: f64 = 2.0;
pub(crate) const STANDARD_BORDER_WIDTH: f64 = 1.0;
pub(crate) const MEDIUM_BORDER_WIDTH: f64 = 2.0;
pub(crate) const THICK_BORDER_WIDTH: f64 = 3.0;
pub(crate) const DASHED_BORDER_WIDTH: f64 = 1.5;

pub const HEADER_OFFSET: f64 = 1.0;
pub const HEADER_ROW_HEIGHT: f64 = 28.0;
pub const HEADER_COL_WIDTH: f64 = 30.0;
pub const FROZEN_SEP: f64 = 3.0;
/// Side length of the autofill handle square. The handle's top-left sits at
/// the selection's bottom-right corner (Excel anchor) so it visually pokes
/// outside the selection rectangle.
pub const AUTOFILL_HANDLE_PX: f64 = 6.0;
/// Width of the contrasting outline ring stroked around the handle. Sourced
/// from `theme.cell_bg` so the handle pops against any cell fill underneath.
pub const AUTOFILL_HANDLE_BORDER_PX: f64 = 1.0;
/// Extra forgiveness around the handle's visual rect on every side when
/// hit-testing pointer events — keeps the click target a couple pixels
/// larger than the painted square.
pub const AUTOFILL_HIT_PAD_PX: f64 = 2.0;

/// Fallback row height when the model returns `None` (row not explicitly sized).
pub const DEFAULT_ROW_HEIGHT: f64 = 21.0;
/// Fallback column width when the model returns `None` (column not explicitly sized).
pub const DEFAULT_COL_WIDTH: f64 = 64.0;
/// Min/Max index (Excel/OOXML limit).
pub const LAST_ROW: i32 = 1_048_576;
pub const LAST_COLUMN: i32 = 16_384;
