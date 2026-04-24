//! Pixel↔cell coordinate math, layout constants, and the `PixelRect` / `Line`
//! primitives that every renderer call eventually bottoms out on.

use std::fmt::{self, Display};
use std::ops::RangeInclusive;

use ironcalc_base::UserModel;

use crate::coord::CellArea;

pub const HEADER_OFFSET: f64 = 0.5;
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

/// Row height for `row` on `sheet`, falling back to `DEFAULT_ROW_HEIGHT`.
#[inline]
pub fn row_height(m: &UserModel, row: i32) -> f64 {
    m.get_row_height(m.get_selected_sheet(), row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
}

/// Column width for `col` on `sheet`, falling back to `DEFAULT_COL_WIDTH`.
#[inline]
pub fn col_width(m: &UserModel, col: i32) -> f64 {
    m.get_column_width(m.get_selected_sheet(), col)
        .unwrap_or(DEFAULT_COL_WIDTH)
}

/// Convert a 1-based column index to its spreadsheet letter name (A, B, ..., XFD).
///
/// Delegates to `ironcalc_base::expressions::utils::number_to_column` - the
/// single authoritative implementation for this conversion in the codebase.
pub fn col_name(col: i32) -> String {
    ironcalc_base::expressions::utils::number_to_column(col).unwrap_or_default()
}

/// Size of the drawable canvas in logical (CSS) pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSize {
    pub w: f64,
    pub h: f64,
}

/// A rectangle in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelRect {
    pub top_left: Point,
    //pub size: Point,
    pub width: f64,
    pub height: f64,
}

impl PixelRect {
    pub fn right(&self) -> f64 {
        self.top_left.x + self.width
    }
    pub fn bottom(&self) -> f64 {
        self.top_left.y + self.height
    }

    #[cfg(test)]
    pub fn top_left(&self) -> Point {
        self.top_left
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.top_left.x + self.width / 2.0,
            y: self.top_left.y + self.height / 2.0,
        }
    }
    /// Shrink by `dx` / `dy` on each side (negative values grow the rect).
    pub fn inset(&self, dx: f64, dy: f64) -> Self {
        Self {
            top_left: Point {
                x: self.top_left.x + dx,
                y: self.top_left.y + dy,
            },

            width: self.width - 2.0 * dx,
            height: self.height - 2.0 * dy,
        }
    }

    /// True when this rect overlaps the canvas drawable area at all.
    /// Pure pixel-space AABB test — used inside per-cell loops to skip cells
    /// that fall off-canvas (notably when a frozen band is wider/taller than
    /// the canvas itself).
    pub fn intersects(&self, canvas: CanvasSize) -> bool {
        self.top_left.x < canvas.w
            && self.right() > 0.0
            && self.top_left.y < canvas.h
            && self.bottom() > 0.0
    }
}

impl Display for PixelRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px;",
            self.top_left.x, self.top_left.y, self.width, self.height
        )
    }
}

/// A point in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// An axis-aligned line segment on the canvas.
///
/// Named fields per variant so callers can't transpose the scalars.
/// `offset_cross` shifts perpendicular to the line's direction — used by
/// `BorderStyle::Double`, which draws two parallel lines at ±1 on the
/// cross-axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    H { span: Span, y: f64 },
    V { x: f64, span: Span },
}

impl Line {
    /// Move the line by `d` perpendicular to its direction.
    pub fn offset_cross(self, d: f64) -> Self {
        match self {
            Line::H { span, y } => Line::H { span, y: y + d },
            Line::V { span, x } => Line::V { span, x: x + d },
        }
    }
}

/// Line length
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub from: f64,
    pub to: f64,
}

/// Pixel origin of the scrollable (non-frozen) grid area.
///
/// Passed to renderer drawing helpers so call sites read
/// `cell_x(model, col, frozen)` without a second unrelated tuple parameter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrozenOffset {
    pub origin: Point,
}

/// Frozen rows and columns grouped with their pixel origin.
///
/// The band shape (`Option<RangeInclusive<i32>>`) is the extension seam: today
/// `from_model` only emits `1..=N` (anchored at the top-left), but the range
/// carries the start index too, so a named-range-anchored freeze becomes a
/// future variant on the shape without touching the four-quadrant math.
///
/// Supersedes the older `FrozenGeometry` (which only knew counts + offsets —
/// a strict subset of what this type carries). Migration is one-way: every
/// `FrozenGeometry` field is derivable from a `FrozenRC`, but the reverse
/// requires an anchor assumption.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenRC {
    pub row_band: Option<RangeInclusive<i32>>,
    pub col_band: Option<RangeInclusive<i32>>,
    pub offset: FrozenOffset,
}

impl FrozenRC {
    /// Read frozen geometry from the currently-selected sheet on `model`.
    pub fn from_model(model: &UserModel) -> Self {
        let sheet = model.get_selected_sheet();
        let rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let h: f64 = (1..=rows).map(|r| row_height(model, r)).sum();
        let w: f64 = (1..=cols).map(|c| col_width(model, c)).sum();
        FrozenRC {
            row_band: (rows > 0).then_some(1..=rows),
            col_band: (cols > 0).then_some(1..=cols),
            offset: FrozenOffset {
                origin: Point {
                    x: HEADER_COL_WIDTH + w + if cols > 0 { FROZEN_SEP } else { 0.0 },
                    y: HEADER_ROW_HEIGHT + h + if rows > 0 { FROZEN_SEP } else { 0.0 },
                },
            },
        }
    }

    /// Count of frozen rows — derived from `row_band`, preserving the
    /// "no band = 0" invariant. Assumes a band ending at `N` represents
    /// `N` frozen entries (true for the `1..=N` anchor used today).
    #[inline]
    pub fn frozen_rows_count(&self) -> i32 {
        self.row_band.as_ref().map_or(0, |r| *r.end())
    }

    /// Count of frozen columns — mirror of `frozen_rows_count`.
    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
        self.col_band.as_ref().map_or(0, |c| *c.end())
    }
}

/// Snapshot of "where cells are drawn right now" for one sheet — the model,
/// the view's scroll anchors, and the frozen-pane pixel splits bundled
/// together.
pub struct SheetViewport<'a> {
    model: &'a UserModel<'a>,
    left_column: i32,
    top_row: i32,
    frozen: FrozenRC,
}

impl<'a> SheetViewport<'a> {
    /// Snapshot the currently-selected sheet with its current scroll state.
    pub fn current(model: &'a UserModel<'a>) -> Self {
        let view = model.get_selected_view();
        Self::from_parts(model, view.left_column, view.top_row)
    }

    /// Build from explicit anchors. Shims whose callers already destructured
    /// the view use this; callers that only need one anchor may pass any value
    /// for the other (the method dispatched will ignore it).
    pub fn from_parts(model: &'a UserModel<'a>, left_column: i32, top_row: i32) -> Self {
        Self {
            model,
            left_column,
            top_row,
            frozen: FrozenRC::from_model(model),
        }
    }

    pub fn sheet(&self) -> u32 {
        self.model.get_selected_sheet()
    }

    pub fn frozen(&self) -> &FrozenRC {
        &self.frozen
    }

    /// Left-edge X pixel of `col` at current scroll.
    pub fn col_to_x(&self, col: i32) -> f64 {
        let frozen_cols = self.frozen.frozen_cols_count();
        if col <= frozen_cols {
            HEADER_COL_WIDTH + (1..col).map(|c| col_width(self.model, c)).sum::<f64>()
        } else {
            let left = self.left_column.max(frozen_cols + 1);
            self.frozen.offset.origin.x + (left..col).map(|c| col_width(self.model, c)).sum::<f64>()
        }
    }

    /// Top-edge Y pixel of `row` at current scroll.
    pub fn row_to_y(&self, row: i32) -> f64 {
        let frozen_rows = self.frozen.frozen_rows_count();
        if row <= frozen_rows {
            HEADER_ROW_HEIGHT + (1..row).map(|r| row_height(self.model, r)).sum::<f64>()
        } else {
            let top = self.top_row.max(frozen_rows + 1);
            self.frozen.offset.origin.y + (top..row).map(|r| row_height(self.model, r)).sum::<f64>()
        }
    }

    /// 1-based column at canvas X pixel `x`.
    pub fn pixel_to_col(&self, x: f64) -> i32 {
        let frozen_cols = self.frozen.frozen_cols_count();
        if x < self.frozen.offset.origin.x {
            let mut cx = HEADER_COL_WIDTH;
            let mut result = 1_i32.max(frozen_cols);
            for c in 1..=frozen_cols {
                let cw = col_width(self.model, c);
                if x < cx + cw {
                    result = c;
                    break;
                }
                cx += cw;
            }
            result
        } else {
            let start = (frozen_cols + 1).max(self.left_column);
            let mut cx = self.frozen.offset.origin.x;
            let mut c = start;
            loop {
                let cw = col_width(self.model, c);
                if x < cx + cw || c >= LAST_COLUMN {
                    break c;
                }
                cx += cw;
                c += 1;
            }
        }
    }

    /// 1-based row at canvas Y pixel `y`.
    pub fn pixel_to_row(&self, y: f64) -> i32 {
        let frozen_rows = self.frozen.frozen_rows_count();
        if y < self.frozen.offset.origin.y {
            let mut cy = HEADER_ROW_HEIGHT;
            let mut result = 1_i32.max(frozen_rows);
            for r in 1..=frozen_rows {
                let rh = row_height(self.model, r);
                if y < cy + rh {
                    result = r;
                    break;
                }
                cy += rh;
            }
            result
        } else {
            let start = (frozen_rows + 1).max(self.top_row);
            let mut cy = self.frozen.offset.origin.y;
            let mut r = start;
            loop {
                let rh = row_height(self.model, r);
                if y < cy + rh || r >= LAST_ROW {
                    break r;
                }
                cy += rh;
                r += 1;
            }
        }
    }

    /// Pixel rectangle for `(row, col)` at current scroll.
    pub fn cell_rect(&self, row: i32, col: i32) -> PixelRect {
        PixelRect {
            top_left: Point {
                x: self.col_to_x(col),
                y: self.row_to_y(row),
            },
            width: col_width(self.model, col),
            height: row_height(self.model, row),
        }
    }

    /// Bottom-right pixel of the current selection range — the autofill handle
    /// anchor. `None` for full-row/column/sheet selections, where `col_to_x` /
    /// `row_to_y` would walk up to 1M cells to produce an off-screen pixel.
    pub fn autofill_handle(&self) -> Option<Point> {
        let area = CellArea::from_view(self.model).normalized();
        if area.r2 >= LAST_ROW || area.c2 >= LAST_COLUMN {
            return None;
        }
        Some(Point {
            x: self.col_to_x(area.c2) + col_width(self.model, area.c2),
            y: self.row_to_y(area.r2) + row_height(self.model, area.r2),
        })
    }

    /// Column whose RIGHT edge is within `hit_zone` px of `x`, or `None`.
    /// Used by mousedown to detect clicks on column-resize handles.
    pub fn col_boundary_at(&self, x: f64, hit_zone: f64) -> Option<i32> {
        let frozen_cols = self.frozen.frozen_cols_count();
        if frozen_cols > 0 {
            let mut cur_x = HEADER_COL_WIDTH;
            for col in 1..=frozen_cols {
                cur_x += col_width(self.model, col);
                if (cur_x - x).abs() <= hit_zone {
                    return Some(col);
                }
            }
        }
        let start = (frozen_cols + 1).max(self.left_column);
        let mut cur_x = self.frozen.offset.origin.x;
        let mut col = start;
        while cur_x < x + hit_zone + 5.0 {
            cur_x += col_width(self.model, col);
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

    /// Row whose BOTTOM edge is within `hit_zone` px of `y`, or `None`.
    /// Used by mousedown to detect clicks on row-resize handles.
    pub fn row_boundary_at(&self, y: f64, hit_zone: f64) -> Option<i32> {
        let frozen_rows = self.frozen.frozen_rows_count();
        if frozen_rows > 0 {
            let mut cur_y = HEADER_ROW_HEIGHT;
            for row in 1..=frozen_rows {
                cur_y += row_height(self.model, row);
                if (cur_y - y).abs() <= hit_zone {
                    return Some(row);
                }
            }
        }
        let start = (frozen_rows + 1).max(self.top_row);
        let mut cur_y = self.frozen.offset.origin.y;
        let mut row = start;
        while cur_y < y + hit_zone + 5.0 {
            cur_y += row_height(self.model, row);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_is_x_plus_width() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(rect.right(), 25.0);
    }

    #[test]
    fn bottom_is_y_plus_height() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 25.0,
        };
        assert_eq!(rect.bottom(), 35.0);
    }

    #[test]
    fn top_left_returns_point_at_rect_origin() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            height: 20.0,
            width: 15.0,
        };
        assert_eq!(rect.top_left(), Point { x: 5.0, y: 10.0 });
    }

    #[test]
    fn center_returns_midpoint() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 10.0 },
            width: 30.0,
            height: 30.0,
        };
        assert_eq!(rect.center(), Point { x: 25.0, y: 25.0 });
    }

    #[test]
    fn inset_with_positive_values_shrinks_symmetrically() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 20.0 },
            width: 100.0,
            height: 50.0,
        };
        let inner = rect.inset(2.0, 3.0);
        assert_eq!(inner.top_left.x, 12.0);
        assert_eq!(inner.top_left.y, 23.0);
        assert_eq!(inner.width, 96.0);
        assert_eq!(inner.height, 44.0);
    }

    #[test]
    fn inset_with_zero_is_identity() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 20.0 },
            width: 100.0,
            height: 50.0,
        };
        let inner = rect.inset(0.0, 0.0);
        assert_eq!(inner.top_left.x, 10.0);
        assert_eq!(inner.top_left.y, 20.0);
        assert_eq!(inner.width, 100.0);
        assert_eq!(inner.height, 50.0);
    }

    #[test]
    fn inset_with_negative_values_grows_rect() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 20.0 },
            width: 100.0,
            height: 50.0,
        };
        let inner = rect.inset(-10.0, -10.0);
        assert_eq!(inner.top_left.x, 0.0);
        assert_eq!(inner.top_left.y, 10.0);
        assert_eq!(inner.width, 120.0);
        assert_eq!(inner.height, 70.0);
    }

    #[test]
    fn inset_preserves_center() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 20.0 },
            width: 100.0,
            height: 50.0,
        };
        let inner = rect.inset(50.0, 100.0);
        assert_eq!(rect.center(), inner.center());
    }

    #[test]
    fn intersects_true_when_rect_inside_canvas() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 10.0 },
            width: 50.0,
            height: 50.0,
        };
        assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
    }

    #[test]
    fn intersects_true_when_rect_straddles_edge() {
        let rect = PixelRect {
            top_left: Point { x: -10.0, y: -10.0 },
            width: 50.0,
            height: 50.0,
        };
        assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
    }

    #[test]
    fn intersects_false_when_rect_off_right() {
        let rect = PixelRect {
            top_left: Point { x: 250.0, y: 10.0 },
            width: 50.0,
            height: 50.0,
        };
        assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
    }

    #[test]
    fn intersects_false_when_rect_off_left() {
        let rect = PixelRect {
            top_left: Point { x: -100.0, y: 10.0 },
            width: 50.0,
            height: 50.0,
        };
        assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
    }

    #[test]
    fn intersects_false_when_rect_below_canvas() {
        let rect = PixelRect {
            top_left: Point { x: 10.0, y: 250.0 },
            width: 50.0,
            height: 50.0,
        };
        assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
    }

    #[test]
    fn horizontal_line_offsets_y_by_delta() {
        //
        let line = Line::H {
            span: Span {
                from: 0.0,
                to: 10.0,
            },
            y: 5.0,
        };
        assert_eq!(
            line.offset_cross(2.0),
            Line::H {
                span: Span {
                    from: 0.0,
                    to: 10.0,
                },
                y: 7.0,
            }
        );
    }

    #[test]
    fn vertical_line_offsets_x_by_delta() {
        let line = Line::V {
            span: Span {
                from: 0.0,
                to: 10.0,
            },
            x: 5.0,
        };
        assert_eq!(
            line.offset_cross(2.0),
            Line::V {
                span: Span {
                    from: 0.0,
                    to: 10.0,
                },
                x: 7.0,
            }
        );
    }

    #[test]
    fn offset_cross_with_zero_is_identity() {
        let span = Span {
            from: 0.0,
            to: 10.0,
        };
        let h = Line::H { span, y: 5.0 };
        assert_eq!(h.offset_cross(0.0), h);
        let v = Line::V { x: 5.0, span };
        assert_eq!(v.offset_cross(0.0), v);
    }

    #[test]
    fn offset_cross_with_negative_shifts_opposite() {
        let span = Span {
            from: 0.0,
            to: 10.0,
        };
        assert_eq!(
            Line::H { span, y: 5.0 }.offset_cross(-2.0),
            Line::H { span, y: 3.0 }
        );
        assert_eq!(
            Line::V { x: 5.0, span }.offset_cross(-2.0),
            Line::V { x: 3.0, span }
        );
    }

    #[test]
    fn col_name_one_is_a() {
        assert_eq!(col_name(1), "A");
    }

    #[test]
    fn col_name_26_is_z() {
        assert_eq!(col_name(26), "Z");
    }

    #[test]
    fn col_name_707_is_zz() {
        assert_eq!(col_name(707), "AAE");
    }

    #[test]
    fn col_name_zero_returns_empty_string() {
        assert_eq!(col_name(0), "");
    }
}
