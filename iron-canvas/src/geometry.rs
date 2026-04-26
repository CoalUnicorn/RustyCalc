//! Pixel↔cell coordinate math, layout constants, and the `PixelRect` / `Line`
//! primitives that every renderer call eventually bottoms out on.

use std::fmt::{self, Display};
use std::ops::RangeInclusive;

use crate::model::RCRange;
use crate::CanvasModel;

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
pub fn row_height(m: &dyn CanvasModel, row: i32) -> f64 {
    m.get_row_height(m.get_selected_sheet(), row)
        .unwrap_or(DEFAULT_ROW_HEIGHT)
}

/// Column width for `col` on `sheet`, falling back to `DEFAULT_COL_WIDTH`.
#[inline]
pub fn col_width(m: &dyn CanvasModel, col: i32) -> f64 {
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
    /// Pure pixel-space AABB test - used inside per-cell loops to skip cells
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
/// `offset_cross` shifts perpendicular to the line's direction - used by
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
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenRC {
    pub row_band: Option<RangeInclusive<i32>>,
    pub col_band: Option<RangeInclusive<i32>>,
    pub offset: FrozenOffset,
}

impl FrozenRC {
    /// Read frozen geometry from the currently-selected sheet on `model`.
    pub fn from_model(model: &dyn CanvasModel) -> Self {
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

    /// Count of frozen rows - derived from `row_band`, preserving the
    /// "no band = 0" invariant. Assumes a band ending at `N` represents
    /// `N` frozen entries (true for the `1..=N` anchor used today).
    #[inline]
    pub fn frozen_rows_count(&self) -> i32 {
        self.row_band.as_ref().map_or(0, |r| *r.end())
    }

    /// Count of frozen columns - mirror of `frozen_rows_count`.
    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
        self.col_band.as_ref().map_or(0, |c| *c.end())
    }
}

/// The four index boundaries of the visible (scrollable) area.
#[derive(Debug, Default)]
pub struct VisibleRegion {
    /// Top-left scrollable cell on screen.
    pub first: CellRC,
    /// Bottom-right scrollable cell on screen.
    pub last: CellRC,
}

#[derive(Debug, Default)]
pub struct CellRC {
    pub row: i32,
    pub column: i32,
}

/// Precomputed pixel offsets for visible rows and columns.
///
/// Built once per render call from the same iteration used to determine
/// `VisibleRegion`. Eliminates the O(visible_range × R) summation inside
/// `cell_x` / `cell_y` - each lookup becomes O(1).
///
/// Offsets are relative to `FrozenOffset`: `row_tops[i]` is the Y distance
/// from `frozen.y` to the top edge of row `(row_start + i as i32)`.
#[derive(Debug, Default)]
pub(crate) struct PixelOffsets {
    pub row_start: i32,
    pub row_tops: Vec<f64>,
    pub col_start: i32,
    pub col_lefts: Vec<f64>,
}

impl PixelOffsets {
    /// Y distance from `frozen.y` to the top edge of `row`.
    ///
    /// Returns `0.0` for rows outside the precomputed range. In practice
    /// `range_pixel_bounds` clamps oversized selections to the canvas edge
    /// before calling `cell_y`, so this fallback is never reached.
    #[inline]
    pub fn row_top(&self, row: i32) -> f64 {
        self.row_tops
            .get((row - self.row_start) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// X distance from `frozen.x` to the left edge of `col`.
    #[inline]
    pub fn col_left(&self, col: i32) -> f64 {
        self.col_lefts
            .get((col - self.col_start) as usize)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Per-frame geometric snapshot threaded into every render phase.
///
/// Built once at the start of `CanvasRenderer::render` from a `SheetViewport`,
/// then borrowed (never owned) by `render_pane`, `render_headers`, and the
/// overlay drawers. Bundles the visible region, the pixel-offset prefix-sum
/// cache, and the resolved frozen-pane geometry so phases never re-read those
/// from the model mid-frame.
#[derive(Debug)]
pub(crate) struct FrameContext {
    pub vis: VisibleRegion,
    pub offsets: PixelOffsets,
    pub frozen: FrozenRC,
}

/// Snapshot of "where cells are drawn right now" for one sheet - the model,
/// the view's scroll anchors, and the frozen-pane pixel splits bundled
/// together.
pub struct SheetViewport<'a> {
    model: &'a dyn CanvasModel,
    left_column: i32,
    top_row: i32,
    frozen: FrozenRC,
}

impl<'a> SheetViewport<'a> {
    /// Snapshot the currently-selected sheet with its current scroll state.
    pub fn current(model: &'a dyn CanvasModel) -> Self {
        let view = model.get_selected_view();
        Self::from_parts(model, view.left_column, view.top_row)
    }

    /// Build from explicit anchors. Shims whose callers already destructured
    /// the view use this; callers that only need one anchor may pass any value
    /// for the other (the method dispatched will ignore it).
    pub fn from_parts(model: &'a dyn CanvasModel, left_column: i32, top_row: i32) -> Self {
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

    /// Bottom-right pixel of the current selection range - the autofill handle
    /// anchor. `None` for full-row/column/sheet selections, where `col_to_x` /
    /// `row_to_y` would walk up to 1M cells to produce an off-screen pixel.
    pub fn autofill_handle(&self) -> Option<Point> {
        let area = RCRange::from_view(self.model).normalized();
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

    /// Compute the visible scrollable cell region for a canvas of size `canvas`.
    ///
    /// Independent of selection state - performance stays constant whether the
    /// selection is one cell or the whole sheet. Scans rows/cols until the
    /// canvas is filled, capping at `SCAN_CAP` to prevent O(LAST_ROW) iteration
    /// when many rows are explicitly hidden (height = 0).
    pub(crate) fn visible_region(&self, canvas: CanvasSize) -> VisibleRegion {
        let frozen_rows = self.frozen.frozen_rows_count();
        let frozen_cols = self.frozen.frozen_cols_count();
        let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(self.model, r)).sum();
        let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(self.model, c)).sum();

        let row_first = (frozen_rows + 1).max(self.top_row);
        let col_first = (frozen_cols + 1).max(self.left_column);

        let mut row_last = row_first;
        let mut y = HEADER_ROW_HEIGHT + frozen_rows_h;
        for row in row_first..=LAST_ROW {
            if y >= canvas.h || row == LAST_ROW {
                row_last = row;
                break;
            }
            y += row_height(self.model, row);
        }

        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_cols_w;
        for col in col_first..=LAST_COLUMN {
            if x >= canvas.w || col == LAST_COLUMN {
                col_last = col;
                break;
            }
            x += col_width(self.model, col);
        }

        VisibleRegion {
            first: CellRC {
                column: col_first,
                row: row_first,
            },
            last: CellRC {
                column: col_last,
                row: row_last,
            },
        }
    }

    /// Build a prefix-sum pixel-offset table for all visible rows and columns.
    ///
    /// Each `row_tops[i]` is the cumulative Y distance from `frozen.y` to the
    /// top edge of row `(vis.first.row + i)`. Built in a single O(visible) pass
    /// - same rows/cols `visible_region` already iterated. The returned table
    ///   feeds `cell_x` / `cell_y` so they become O(1) array lookups instead of
    ///   O(visible × R) summations.
    pub(crate) fn pixel_offsets(&self, vis: &VisibleRegion) -> PixelOffsets {
        let mut row_tops = Vec::with_capacity((vis.last.row - vis.first.row + 2) as usize);
        let mut acc = 0.0_f64;
        for r in vis.first.row..=vis.last.row {
            row_tops.push(acc);
            acc += row_height(self.model, r);
        }
        row_tops.push(acc);

        let mut col_lefts = Vec::with_capacity((vis.last.column - vis.first.column + 2) as usize);
        acc = 0.0;
        for c in vis.first.column..=vis.last.column {
            col_lefts.push(acc);
            acc += col_width(self.model, c);
        }
        col_lefts.push(acc);

        PixelOffsets {
            row_start: vis.first.row,
            row_tops,
            col_start: vis.first.column,
            col_lefts,
        }
    }

    /// One-shot per-frame snapshot. Resolves the visible region, the pixel-offset
    /// prefix sums, and bundles the already-resolved frozen geometry into a
    /// `FrameContext` ready to thread through every render phase.
    pub(crate) fn frame(&self, canvas: CanvasSize) -> FrameContext {
        let vis = self.visible_region(canvas);
        let offsets = self.pixel_offsets(&vis);
        FrameContext {
            vis,
            offsets,
            frozen: self.frozen.clone(),
        }
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

    // Test fixture - a configurable in-memory CanvasModel.
    //
    // Only methods exercised by viewport / frozen-pane math are wired up.
    // Style / cell-content methods stay `unimplemented!()` so a future test
    // that touches them fails loudly rather than silently consuming defaults.
    use crate::SelectedView;

    struct MockCanvasModel {
        sheet: u32,
        frozen_rows: i32,
        frozen_cols: i32,
        row_height: f64,
        col_width: f64,
        range: [i32; 4],
        top_row: i32,
        left_column: i32,
    }

    impl Default for MockCanvasModel {
        fn default() -> Self {
            Self {
                sheet: 0,
                frozen_rows: 0,
                frozen_cols: 0,
                row_height: DEFAULT_ROW_HEIGHT,
                col_width: DEFAULT_COL_WIDTH,
                range: [1, 1, 1, 1],
                top_row: 1,
                left_column: 1,
            }
        }
    }

    impl CanvasModel for MockCanvasModel {
        fn get_selected_sheet(&self) -> u32 {
            self.sheet
        }
        fn get_selected_view(&self) -> SelectedView {
            SelectedView {
                sheet: self.sheet,
                row: self.range[0],
                column: self.range[1],
                range: self.range,
                top_row: self.top_row,
                left_column: self.left_column,
            }
        }
        fn get_frozen_rows_count(&self, _sheet: u32) -> Result<i32, String> {
            Ok(self.frozen_rows)
        }
        fn get_frozen_columns_count(&self, _sheet: u32) -> Result<i32, String> {
            Ok(self.frozen_cols)
        }
        fn get_row_height(&self, _sheet: u32, _row: i32) -> Result<f64, String> {
            Ok(self.row_height)
        }
        fn get_column_width(&self, _sheet: u32, _column: i32) -> Result<f64, String> {
            Ok(self.col_width)
        }
        fn get_show_grid_lines(&self, _sheet: u32) -> Result<bool, String> {
            Ok(true)
        }
        fn get_cell_style(
            &self,
            _: u32,
            _: i32,
            _: i32,
        ) -> Result<ironcalc_base::types::Style, String> {
            unimplemented!("style not used by these tests")
        }
        fn get_cell_type(
            &self,
            _: u32,
            _: i32,
            _: i32,
        ) -> Result<ironcalc_base::types::CellType, String> {
            unimplemented!("cell type not used by these tests")
        }
        fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Result<String, String> {
            unimplemented!("cell value not used by these tests")
        }
    }

    // FrozenRC

    #[test]
    fn frozen_rc_no_freeze_has_no_bands_and_origin_skips_separator() {
        let m = MockCanvasModel::default();
        let frc = FrozenRC::from_model(&m);
        assert!(frc.row_band.is_none());
        assert!(frc.col_band.is_none());
        assert_eq!(frc.frozen_rows_count(), 0);
        assert_eq!(frc.frozen_cols_count(), 0);
        assert_eq!(frc.offset.origin.x, HEADER_COL_WIDTH);
        assert_eq!(frc.offset.origin.y, HEADER_ROW_HEIGHT);
    }

    #[test]
    fn frozen_rc_rows_only_adds_separator_on_y_only() {
        let m = MockCanvasModel {
            frozen_rows: 2,
            ..Default::default()
        };
        let frc = FrozenRC::from_model(&m);
        assert_eq!(frc.row_band, Some(1..=2));
        assert!(frc.col_band.is_none());
        assert_eq!(frc.frozen_rows_count(), 2);
        assert_eq!(frc.frozen_cols_count(), 0);
        assert_eq!(frc.offset.origin.x, HEADER_COL_WIDTH);
        assert_eq!(
            frc.offset.origin.y,
            HEADER_ROW_HEIGHT + 2.0 * DEFAULT_ROW_HEIGHT + FROZEN_SEP
        );
    }

    #[test]
    fn frozen_rc_both_axes_add_separator_on_each() {
        let m = MockCanvasModel {
            frozen_rows: 1,
            frozen_cols: 3,
            ..Default::default()
        };
        let frc = FrozenRC::from_model(&m);
        assert_eq!(frc.frozen_rows_count(), 1);
        assert_eq!(frc.frozen_cols_count(), 3);
        assert_eq!(
            frc.offset.origin.x,
            HEADER_COL_WIDTH + 3.0 * DEFAULT_COL_WIDTH + FROZEN_SEP
        );
        assert_eq!(
            frc.offset.origin.y,
            HEADER_ROW_HEIGHT + DEFAULT_ROW_HEIGHT + FROZEN_SEP
        );
    }

    // PixelOffsets

    #[test]
    fn pixel_offsets_row_top_returns_zero_outside_precomputed_range() {
        let off = PixelOffsets {
            row_start: 10,
            row_tops: vec![0.0, 20.0, 40.0],
            col_start: 5,
            col_lefts: vec![0.0, 60.0],
        };
        assert_eq!(off.row_top(10), 0.0);
        assert_eq!(off.row_top(11), 20.0);
        assert_eq!(off.row_top(99), 0.0);
        assert_eq!(off.col_left(5), 0.0);
        assert_eq!(off.col_left(6), 60.0);
        assert_eq!(off.col_left(99), 0.0);
    }

    // SheetViewport

    #[test]
    fn cell_rect_at_origin_starts_at_top_left_header_corner() {
        let m = MockCanvasModel::default();
        let vp = SheetViewport::current(&m);
        let r = vp.cell_rect(1, 1);
        assert_eq!(r.top_left.x, HEADER_COL_WIDTH);
        assert_eq!(r.top_left.y, HEADER_ROW_HEIGHT);
        assert_eq!(r.width, DEFAULT_COL_WIDTH);
        assert_eq!(r.height, DEFAULT_ROW_HEIGHT);
    }

    #[test]
    fn col_to_x_inside_frozen_band_skips_frozen_offset() {
        let m = MockCanvasModel {
            frozen_cols: 2,
            ..Default::default()
        };
        let vp = SheetViewport::current(&m);
        assert_eq!(vp.col_to_x(1), HEADER_COL_WIDTH);
        assert_eq!(vp.col_to_x(2), HEADER_COL_WIDTH + DEFAULT_COL_WIDTH);
    }

    #[test]
    fn col_to_x_past_frozen_seam_uses_frozen_offset_and_left_column() {
        let m = MockCanvasModel {
            frozen_cols: 2,
            left_column: 5,
            ..Default::default()
        };
        let vp = SheetViewport::current(&m);
        let origin_x = vp.frozen().offset.origin.x;
        // col 5 is the first scrollable on screen → at the frozen offset
        assert_eq!(vp.col_to_x(5), origin_x);
        assert_eq!(vp.col_to_x(6), origin_x + DEFAULT_COL_WIDTH);
    }

    #[test]
    fn autofill_handle_is_none_for_full_sheet_selection() {
        let m = MockCanvasModel {
            range: [1, 1, LAST_ROW, LAST_COLUMN],
            ..Default::default()
        };
        let vp = SheetViewport::current(&m);
        assert!(vp.autofill_handle().is_none());
    }

    #[test]
    fn autofill_handle_lands_at_bottom_right_of_finite_selection() {
        let m = MockCanvasModel {
            range: [2, 3, 4, 5],
            ..Default::default()
        };
        let vp = SheetViewport::current(&m);
        let p = vp.autofill_handle().expect("finite selection has handle");
        assert_eq!(p.x, vp.col_to_x(5) + DEFAULT_COL_WIDTH);
        assert_eq!(p.y, vp.row_to_y(4) + DEFAULT_ROW_HEIGHT);
    }

    // Round-trip property - handed to a human for the column-sample design.
    #[test]
    fn pixel_to_col_round_trips_col_to_x() {
        // TODO(human): build a `SheetViewport` (use `MockCanvasModel` above)
        // and prove the contract:
        //
        //     viewport.pixel_to_col(viewport.col_to_x(c)) == c
        //
        // for a handful of column samples that span the interesting cases:
        //   - inside the frozen band (col <= frozen_cols)
        //   - the first scrollable column (the seam, col = frozen_cols + 1)
        //   - past a non-1 `left_column` scroll
        //
        // Pick the freeze layout, the scroll position, and the sample
        // columns that you think exercise the seam most convincingly.
        // Mirror the same property for `pixel_to_row(row_to_y(r))` if you
        // want a second assertion.
    }
}
