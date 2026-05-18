// Layout constants
pub(crate) const SELECTION_BORDER_WIDTH: i32 = 2;
pub(crate) const STANDARD_BORDER_WIDTH: i32 = 1;
pub(crate) const MEDIUM_BORDER_WIDTH: i32 = 2;
pub(crate) const THICK_BORDER_WIDTH: i32 = 3;
pub(crate) const DASHED_BORDER_WIDTH: i32 = 2;

pub const HEADER_OFFSET: i32 = 1;
pub const HEADER_ROW_HEIGHT: i32 = 28;
pub const HEADER_COL_WIDTH: i32 = 30;
pub const FROZEN_SEP: i32 = 3;

/// Pixel offset from a header strip's outer edge to the cell area origin.
/// `HEADER_OFFSET` reserves the 1-px chrome border line (`draw_corner_box`
/// strokes it sharply at `header_thickness + 0.5`); the extra
/// `SELECTION_BORDER_WIDTH / 2` reserves a 1-px breathing buffer between
/// the chrome border and the cell area so the selection — which
/// `draw_selection` insets by `SELECTION_BORDER_WIDTH / 2` to keep the
/// centered stroke inside the cell — paints with a visible gap from
/// chrome at row 1 / col A.
pub(crate) const CELL_AREA_INSET: i32 = HEADER_OFFSET;
/// Side length of the autofill handle square. The handle's top-left sits at
/// the selection's bottom-right corner (Excel anchor) so it visually pokes
/// outside the selection rectangle.
pub const AUTOFILL_HANDLE_PX: i32 = 6;
/// Width of the contrasting outline ring stroked around the handle. Sourced
/// from `theme.cell_bg` so the handle pops against any cell fill underneath.
pub const AUTOFILL_HANDLE_BORDER_PX: i32 = 1;
/// Padding added on every side of the handle's visual rect when hit-testing
/// pointer events, so the click target is larger than the painted square.
pub const AUTOFILL_HIT_PAD_PX: i32 = 2;

/// Fallback row height when the model returns `None` (row not explicitly sized).
pub const DEFAULT_ROW_HEIGHT: f64 = 21.0;
/// Fallback column width when the model returns `None` (column not explicitly sized).
pub const DEFAULT_COL_WIDTH: f64 = 64.0;
/// Min/Max index (Excel/OOXML limit).
pub const LAST_ROW: i32 = 1_048_576;
pub const LAST_COLUMN: i32 = 16_384;
