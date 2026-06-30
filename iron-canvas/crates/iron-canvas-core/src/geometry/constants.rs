// Layout constants
pub const SELECTION_BORDER_WIDTH: i32 = 2;
pub const STANDARD_BORDER_WIDTH: i32 = 1;
pub const MEDIUM_BORDER_WIDTH: i32 = 2;
pub const THICK_BORDER_WIDTH: i32 = 3;
pub const DASHED_BORDER_WIDTH: i32 = 2;

/// Width of the 1-px line separating a header strip from the cell area,
/// stroked sharply by `draw_corner_box` at `header_thickness + 0.5`. The
/// single source of truth for the header<->cell boundary: header strips and the
/// corner box fill `[0, thickness)`, this line occupies the next pixel, and
/// the cell area begins one separator-width past the thickness (`CELL_AREA_INSET`).
pub const HEADER_SEPARATOR_WIDTH: i32 = STANDARD_BORDER_WIDTH;

/// Height of the column-header strip in pixels. Static — the strip never
/// resizes to fit content.
pub const HEADER_ROW_HEIGHT: i32 = 28;

/// Minimum width of the row-header strip in pixels. `measure_row_header_width`
/// floors `Chrome.row_header_thickness` at this value so labels under
/// three digits never shrink the strip.
pub const HEADER_COL_WIDTH: i32 = 30;

/// Frozen-pane separator thickness in pixels. Used both as the stroke
/// width drawn by `draw_frozen_separators` and as the gap that
/// `pane_set` reserves between the frozen and scrolling pane bands.
pub const FROZEN_SEP: i32 = 3;

/// Pixel offset from a header strip's outer edge (`header_thickness`) to the
/// cell area origin: `cell_origin = header_thickness + CELL_AREA_INSET`. This
/// is exactly the separator line's width — the strip fills `[0, thickness)`,
/// the separator occupies the single pixel at `thickness`, and the cell area
/// starts immediately after at `thickness + HEADER_SEPARATOR_WIDTH`.
pub const CELL_AREA_INSET: i32 = HEADER_SEPARATOR_WIDTH;
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

/// Inward tolerance for classifying a formula-ref hit as an edge or corner.
/// Wider than `AUTOFILL_HIT_PAD_PX` because the ref overlay has no visible
/// handle to anchor the cursor — the grab zone *is* the tolerance band.
pub const REF_HANDLE_HIT_PAD_PX: i32 = 8;

/// Fallback row height when the model returns `None` (row not explicitly sized).
pub const DEFAULT_ROW_HEIGHT: f64 = 21.0;
/// Fallback column width when the model returns `None` (column not explicitly sized).
pub const DEFAULT_COL_WIDTH: f64 = 64.0;
/// Maximum row index (Excel/OOXML limit).
pub const LAST_ROW: i32 = 1_048_576;
/// Maximum column index (Excel/OOXML limit).
pub const LAST_COLUMN: i32 = 16_384;
