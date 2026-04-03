/// Shared pixel↔cell coordinate math for the spreadsheet canvas.
use ironcalc_base::UserModel;

// Layout constants

pub const HEADER_ROW_HEIGHT: f64 = 28.0;
pub const HEADER_COL_WIDTH: f64 = 30.0;
pub const FROZEN_SEP: f64 = 3.0;
/// Half-side of the autofill handle square drawn at the range's bottom-right.
pub const AUTOFILL_HANDLE_PX: f64 = 6.0;

/// Fallback row height when the model returns `None` (row not explicitly sized).
pub const DEFAULT_ROW_HEIGHT: f64 = 21.0;
/// Fallback column width when the model returns `None` (column not explicitly sized).
pub const DEFAULT_COL_WIDTH: f64 = 64.0;
/// Min/Max index (Excel/OOXML limit).
pub const LAST_ROW: i32 = 1_048_576;
pub const LAST_COLUMN: i32 = 16_384;

// Dimension helpers

/// Row height for `row` on `sheet`, falling back to `DEFAULT_ROW_HEIGHT`.
#[inline]
pub fn row_height(m: &UserModel, sheet: u32, row: i32) -> f64 {
    m.get_row_height(sheet, row).unwrap_or(DEFAULT_ROW_HEIGHT)
}

/// Column width for `col` on `sheet`, falling back to `DEFAULT_COL_WIDTH`.
#[inline]
pub fn col_width(m: &UserModel, sheet: u32, col: i32) -> f64 {
    m.get_column_width(sheet, col).unwrap_or(DEFAULT_COL_WIDTH)
}

// Pixel rectangle

/// A point in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A rectangle in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// FrozenGeometry

/// Pre-computed pixel extents of the frozen-pane region for one sheet.
pub struct FrozenGeometry {
    pub frozen_rows: i32,
    pub frozen_cols: i32,
    /// X pixel where the scrollable column area begins.
    pub frozen_x: f64,
    /// Y pixel where the scrollable row area begins.
    pub frozen_y: f64,
}

/// Compute frozen-pane geometry for `sheet` from `m`.
pub fn frozen_geometry(m: &UserModel, sheet: u32) -> FrozenGeometry {
    let frozen_rows = m.get_frozen_rows_count(sheet).unwrap_or(0);
    let frozen_cols = m.get_frozen_columns_count(sheet).unwrap_or(0);
    let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(m, sheet, r)).sum();
    let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(m, sheet, c)).sum();
    let sep_y = if frozen_rows > 0 { FROZEN_SEP } else { 0.0 };
    let sep_x = if frozen_cols > 0 { FROZEN_SEP } else { 0.0 };
    FrozenGeometry {
        frozen_rows,
        frozen_cols,
        frozen_x: HEADER_COL_WIDTH + frozen_cols_w + sep_x,
        frozen_y: HEADER_ROW_HEIGHT + frozen_rows_h + sep_y,
    }
}

// pixel -> cell

/// Convert a canvas X pixel (from `offset_x`) to a 1-based column index.
///
/// `left_column` is `view.left_column` - the first scrollable column visible.
pub fn pixel_to_col(
    m: &UserModel,
    sheet: u32,
    left_column: i32,
    x: f64,
    fg: &FrozenGeometry,
) -> i32 {
    if x < fg.frozen_x {
        // Inside the frozen-column strip
        let mut cx = HEADER_COL_WIDTH;
        let mut result = 1_i32.max(fg.frozen_cols); // fallback: last frozen col (min 1)
        for c in 1..=fg.frozen_cols {
            let cw = col_width(m, sheet, c);
            if x < cx + cw {
                result = c;
                break;
            }
            cx += cw;
        }
        result
    } else {
        // Inside the scrollable column area
        let start = (fg.frozen_cols + 1).max(left_column);
        let mut cx = fg.frozen_x;
        let mut c = start;
        loop {
            let cw = col_width(m, sheet, c);
            if x < cx + cw || c >= LAST_COLUMN {
                break c;
            }
            cx += cw;
            c += 1;
        }
    }
}

/// Convert a canvas Y pixel (from `offset_y`) to a 1-based row index.
///
/// `top_row` is `view.top_row` - the first scrollable row visible.
pub fn pixel_to_row(m: &UserModel, sheet: u32, top_row: i32, y: f64, fg: &FrozenGeometry) -> i32 {
    if y < fg.frozen_y {
        // Inside the frozen-row strip
        let mut cy = HEADER_ROW_HEIGHT;
        let mut result = 1_i32.max(fg.frozen_rows); // fallback: last frozen row (min 1)
        for r in 1..=fg.frozen_rows {
            let rh = row_height(m, sheet, r);
            if y < cy + rh {
                result = r;
                break;
            }
            cy += rh;
        }
        result
    } else {
        // Inside the scrollable row area
        let start = (fg.frozen_rows + 1).max(top_row);
        let mut cy = fg.frozen_y;
        let mut r = start;
        loop {
            let rh = row_height(m, sheet, r);
            if y < cy + rh || r >= LAST_ROW {
                break r;
            }
            cy += rh;
            r += 1;
        }
    }
}

// cell -> pixel

/// Return the left-edge X pixel of `col` given current scroll state.
///
/// `left_column` is `view.left_column`.
pub fn col_to_x(m: &UserModel, sheet: u32, left_column: i32, col: i32, fg: &FrozenGeometry) -> f64 {
    if col <= fg.frozen_cols {
        let mut x = HEADER_COL_WIDTH;
        for c in 1..col {
            x += col_width(m, sheet, c);
        }
        x
    } else {
        let left_col = left_column.max(fg.frozen_cols + 1);
        let mut x = fg.frozen_x;
        for c in left_col..col {
            x += col_width(m, sheet, c);
        }
        x
    }
}

/// Return the top-edge Y pixel of `row` given current scroll state.
///
/// `top_row` is `view.top_row`.
pub fn row_to_y(m: &UserModel, sheet: u32, top_row: i32, row: i32, fg: &FrozenGeometry) -> f64 {
    if row <= fg.frozen_rows {
        let mut y = HEADER_ROW_HEIGHT;
        for r in 1..row {
            y += row_height(m, sheet, r);
        }
        y
    } else {
        let top = top_row.max(fg.frozen_rows + 1);
        let mut y = fg.frozen_y;
        for r in top..row {
            y += row_height(m, sheet, r);
        }
        y
    }
}

// Convenience helpers

/// Pixel rectangle for the currently selected cell, accounting for frozen
/// panes and scroll position.
pub fn selected_cell_rect(m: &UserModel) -> PixelRect {
    let view = m.get_selected_view();
    let sheet = view.sheet;
    let fg = frozen_geometry(m, sheet);
    PixelRect {
        x: col_to_x(m, sheet, view.left_column, view.column, &fg),
        y: row_to_y(m, sheet, view.top_row, view.row, &fg),
        width: col_width(m, sheet, view.column),
        height: row_height(m, sheet, view.row),
    }
}

/// Bottom-right pixel corner of the current selection range, used for
/// autofill handle hit-testing.
pub fn autofill_handle_pos(m: &UserModel) -> Point {
    let view = m.get_selected_view();
    let sheet = view.sheet;
    let fg = frozen_geometry(m, sheet);
    let [r1, c1, r2, c2] = view.range;
    // Normalise: the active corner can be above/left of the anchor on drag.
    let r2 = r1.max(r2);
    let c2 = c1.max(c2);
    // Full-row / full-column / whole-sheet selections span LAST_ROW or LAST_COLUMN.
    // col_to_x / row_to_y would iterate up to 1M rows to compute an off-screen
    // pixel - skip it and return a sentinel that can never match a hit-test.
    if r2 >= LAST_ROW || c2 >= LAST_COLUMN {
        return Point {
            x: -100.0,
            y: -100.0,
        };
    }
    Point {
        x: col_to_x(m, sheet, view.left_column, c2, &fg) + col_width(m, sheet, c2),
        y: row_to_y(m, sheet, view.top_row, r2, &fg) + row_height(m, sheet, r2),
    }
}

// Header boundary hit-testing

/// Returns the 1-based column index whose RIGHT edge is within `hit_zone` px of
/// `x`, searching from the first scrolled column. Returns `None` if no boundary
/// is close enough to snap to.
///
/// Used by mousedown to decide whether the user is clicking a resize handle in
/// the column header row.
pub fn find_col_boundary_at(m: &UserModel, x: f64, hit_zone: f64) -> Option<i32> {
    let view = m.get_selected_view();
    let sheet = view.sheet;
    let fg = frozen_geometry(m, sheet);
    let scroll_col = view.left_column;
    // Check frozen-column right-edge boundaries first.
    if fg.frozen_cols > 0 {
        let mut cur_x = HEADER_COL_WIDTH;
        for col in 1..=fg.frozen_cols {
            cur_x += col_width(m, sheet, col);
            if (cur_x - x).abs() <= hit_zone {
                return Some(col);
            }
        }
    }
    let start = (fg.frozen_cols + 1).max(scroll_col);
    let mut cur_x = fg.frozen_x;
    let mut col = start;
    // Walk scrollable columns until their right edge is well past the cursor.
    while cur_x < x + hit_zone + 5.0 {
        cur_x += col_width(m, sheet, col);
        if (cur_x - x).abs() <= hit_zone {
            return Some(col);
        }
        if cur_x > x + hit_zone {
            break;
        }
        col += 1;
        if col > LAST_COLUMN {
            break;
        }
    }
    None
}

/// Returns the 1-based row index whose BOTTOM edge is within `hit_zone` px of
/// `y`, searching from the first scrolled row. Returns `None` if no boundary
/// is close enough to snap to.
///
/// Used by mousedown to decide whether the user is clicking a resize handle in
/// the row header column.
pub fn find_row_boundary_at(m: &UserModel, y: f64, hit_zone: f64) -> Option<i32> {
    let view = m.get_selected_view();
    let sheet = view.sheet;
    let fg = frozen_geometry(m, sheet);
    let scroll_row = view.top_row;
    // Check frozen-row bottom-edge boundaries first.
    if fg.frozen_rows > 0 {
        let mut cur_y = HEADER_ROW_HEIGHT;
        for row in 1..=fg.frozen_rows {
            cur_y += row_height(m, sheet, row);
            if (cur_y - y).abs() <= hit_zone {
                return Some(row);
            }
        }
    }
    let start = (fg.frozen_rows + 1).max(scroll_row);
    let mut cur_y = fg.frozen_y;
    let mut row = start;
    // Walk scrollable rows until their bottom edge is well past the cursor.
    while cur_y < y + hit_zone + 5.0 {
        cur_y += row_height(m, sheet, row);
        if (cur_y - y).abs() <= hit_zone {
            return Some(row);
        }
        if cur_y > y + hit_zone {
            break;
        }
        row += 1;
        if row > LAST_ROW {
            break;
        }
    }
    None
}

// Column number -> letter name (A, B, …, AA, …)

/// Convert a 1-based column index to its spreadsheet letter name (A, B, …, XFD).
///
/// Delegates to `ironcalc_base::expressions::utils::number_to_column` - the
/// single authoritative implementation for this conversion in the codebase.
pub fn col_name(col: i32) -> String {
    ironcalc_base::expressions::utils::number_to_column(col).unwrap_or_default()
}
