//! Pixel↔cell coordinate math, layout constants, and the `PixelRect` / `Line`
//! primitives that every renderer call eventually bottoms out on.

use std::fmt::{self, Display};
use std::ops::RangeInclusive;

use crate::model::RCRange;
use crate::{CanvasModel, HitTest, ResizeTarget};

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

impl CanvasSize {
    /// Physical backing-store dimensions from CSS size and DPR.
    /// Truncates fractional pixels — matches browser canvas rounding behaviour.
    pub(crate) fn to_backing_size(self, dpr: f64) -> (u32, u32) {
        ((self.w * dpr) as u32, (self.h * dpr) as u32)
    }
}

//  Shared axis - row-vs-column symmetry

/// Horizontal vs vertical axis.
///
/// Shared across viewport offset math (`cell_offset` dispatches on axis) and
/// header rect building (`Axis::header_rect`). Carries no payload - the
/// row/column index travels as a separate parameter so the same enum value
/// can be used across call sites that don't care about a specific index.
#[derive(Copy, Clone)]
pub(crate) enum Axis {
    Row,
    Column,
}

impl Axis {
    /// Rect that pins a header cell to the corresponding header strip.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols). The cross-axis extent is always the header strip width/height.
    pub(crate) fn header_rect(self, along: f64, height: f64) -> PixelRect {
        match self {
            Axis::Row => PixelRect {
                top_left: Point {
                    x: HEADER_OFFSET,
                    y: along,
                },
                width: HEADER_COL_WIDTH,
                height,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: along,
                    y: HEADER_OFFSET,
                },
                width: height,
                height: HEADER_ROW_HEIGHT,
            },
        }
    }

    /// Extent from the frame's prefix-sum snapshot — zero model access.
    pub(crate) fn frame_extent(self, frame: &FrameContext, index: i32) -> f64 {
        match self {
            Axis::Row => frame.row_extent_at(index),
            Axis::Column => frame.col_extent_at(index),
        }
    }

    /// Pixel position where the header strip begins along this axis,
    /// offset by HEADER_OFFSET `0.5` for crisp integer-coordinate strokes.
    pub(crate) fn strip_start(self) -> f64 {
        match self {
            Axis::Row => HEADER_ROW_HEIGHT + HEADER_OFFSET,
            Axis::Column => HEADER_COL_WIDTH + HEADER_OFFSET,
        }
    }

    /// Visible scrollable band in this axis, drawn from `VisibleRegion`.
    pub(crate) fn visible_band(self, vis: &VisibleCells) -> RangeInclusive<i32> {
        match self {
            Axis::Row => vis.first.row..=vis.last.row,
            Axis::Column => vis.first.column..=vis.last.column,
        }
    }

    /// Count of frozen cells along this axis (0 when nothing is frozen).
    pub(crate) fn frozen_count(self, frame: &FrameContext) -> i32 {
        match self {
            Axis::Row => frame.frozen.rows,
            Axis::Column => frame.frozen.cols,
        }
    }

    /// Pixel origin where the scrollable strip for this axis begins.
    pub(crate) fn frozen_origin(self, frame: &FrameContext) -> f64 {
        match self {
            Axis::Row => frame.frozen.offset.y,
            Axis::Column => frame.frozen.offset.x,
        }
    }

    /// Inclusive `(start, end)` of the user's selection along this axis,
    /// read from ironcalc's `SelectedView.range`
    pub(crate) fn selection_range(self, view_range: RCRange) -> (i32, i32) {
        let (start, end) = match self {
            Axis::Row => (view_range.normalized().r1, view_range.normalized().c1),
            Axis::Column => (view_range.normalized().r2, view_range.normalized().c2),
        };
        (start, end)
    }
}

/// Which edge of a cell rectangle is being stroked.
///
/// `line()` projects the edge onto a `PixelRect` to produce the
/// axis-aligned `Line` segment painted by `paint_border`.
#[derive(Copy, Clone)]
pub(super) enum BorderEdge {
    Left,
    Top,
    Right,
    Bottom,
}

impl BorderEdge {
    /// The axis-aligned `Line` this edge would stroke on `rect`.
    pub fn line(self, rect: PixelRect) -> Line {
        let PixelRect {
            top_left: Point { x, y },
            width,
            height,
        } = rect;
        match self {
            BorderEdge::Left => Line::V {
                x,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Top => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y,
            },
            BorderEdge::Right => Line::V {
                x: x + width,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Bottom => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y: y + height,
            },
        }
    }
}

/// A rectangle in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
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

/// Frozen rows and columns grouped with their pixel origin.
///
/// `rows` / `cols` are counts: today every freeze is anchored at the top-left
/// so `1..=rows` / `1..=cols` is the full extent. A future named-range-anchored
/// freeze would replace these counts with a richer shape.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenRC {
    pub rows: i32,
    pub cols: i32,
    pub offset: Point,
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
            rows,
            cols,
            offset: Point {
                x: HEADER_COL_WIDTH + w + if cols > 0 { FROZEN_SEP } else { 0.0 },
                y: HEADER_ROW_HEIGHT + h + if rows > 0 { FROZEN_SEP } else { 0.0 },
            },
        }
    }

    #[inline]
    pub fn frozen_rows_count(&self) -> i32 {
        self.rows
    }

    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
        self.cols
    }
}

/// The four index boundaries of the visible (scrollable) area.
#[derive(Debug, Default)]
pub struct VisibleCells {
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

/// Precomputed pixel offsets for the painted frame.
///
/// Built once per render call alongside `VisibleRegion`. Every geometry
/// query (`col_to_x`, `pixel_to_col`, `cell_rect`, …) reads from here, not
/// from the model — so a hit-test always sees exactly what the renderer
/// painted on this tick.
///
/// Two cumulative tables, each with one trailing entry so deltas yield
/// per-cell extents:
///
/// * `row_tops` / `col_lefts` cover the **scrollable** band, relative to
///   `FrozenRC::offset`. Length = `visible_count + 1`.
/// * `frozen_row_tops` / `frozen_col_lefts` cover the **frozen** band,
///   relative to the header strip (i.e. start at `0.0`, indexed by
///   `frozen_index - 1`). Length = `frozen_count + 1`.
#[derive(Debug, Default)]
pub(crate) struct PixelOffsets {
    pub row_start: i32,
    pub row_tops: Vec<f64>,
    pub col_start: i32,
    pub col_lefts: Vec<f64>,
    pub frozen_row_tops: Vec<f64>,
    pub frozen_col_lefts: Vec<f64>,
}

impl PixelOffsets {
    /// Y distance from `frozen.y` to the top edge of visible-band `row`.
    ///
    /// Returns `0.0` for rows outside the precomputed range.
    #[inline]
    pub fn row_top(&self, row: i32) -> f64 {
        self.row_tops
            .get((row - self.row_start) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// X distance from `frozen.x` to the left edge of visible-band `col`.
    #[inline]
    pub fn col_left(&self, col: i32) -> f64 {
        self.col_lefts
            .get((col - self.col_start) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// Y distance from the column-header strip to the top of frozen `row`
    /// (1-based, must be ≤ frozen-rows count). Returns `0.0` for rows
    /// outside the cached range — caller is expected to gate on the frozen
    /// band before calling.
    #[inline]
    pub fn frozen_row_top(&self, row: i32) -> f64 {
        self.frozen_row_tops
            .get((row - 1) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// X distance from the row-header strip to the left of frozen `col`.
    #[inline]
    pub fn frozen_col_left(&self, col: i32) -> f64 {
        self.frozen_col_lefts
            .get((col - 1) as usize)
            .copied()
            .unwrap_or(0.0)
    }

    /// Height of the visible-band row at `row`, derived from cumulative deltas.
    /// `0.0` if `row` is outside the visible range.
    #[inline]
    pub fn row_extent(&self, row: i32) -> f64 {
        let i = (row - self.row_start) as usize;
        match (self.row_tops.get(i), self.row_tops.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        }
    }

    /// Width of the visible-band column at `col`.
    #[inline]
    pub fn col_extent(&self, col: i32) -> f64 {
        let i = (col - self.col_start) as usize;
        match (self.col_lefts.get(i), self.col_lefts.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        }
    }

    /// Height of the frozen-band row at `row` (1-based).
    #[inline]
    pub fn frozen_row_extent(&self, row: i32) -> f64 {
        let i = (row - 1) as usize;
        match (self.frozen_row_tops.get(i), self.frozen_row_tops.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        }
    }

    /// Width of the frozen-band column at `col`.
    #[inline]
    pub fn frozen_col_extent(&self, col: i32) -> f64 {
        let i = (col - 1) as usize;
        match (
            self.frozen_col_lefts.get(i),
            self.frozen_col_lefts.get(i + 1),
        ) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        }
    }
}

/// Per-frame geometric snapshot threaded into every render phase AND every
/// hit-test query.
///
/// Built once per tick by `FrameContext::current(model, canvas)` — both the
/// renderer (`paint_if_dirty`) and the input layer (`IronCanvas::hit_test`,
/// `cell_rect`, `resize_handle_at`) read the same snapshot, so what's painted
/// and what gets hit always agree. Bundles the visible region, the
/// pixel-offset prefix-sum cache, and the resolved frozen-pane geometry so
/// neither phase re-reads them from the model mid-frame.
#[derive(Debug)]
pub(crate) struct FrameContext {
    pub vis: VisibleCells,
    pub offsets: PixelOffsets,
    pub frozen: FrozenRC,
    pub top_row: i32, // from model.get_selected_view() — used by orchestrator change detection
    pub left_column: i32,
    /// Active selection at paint time, raw `[r1, c1, r2, c2]` from
    /// `SelectedView.range`. Snapshotting it here keeps `autofill_handle`
    /// pure (no model read) and pins the handle position to the *painted*
    /// selection, even if the model's selection mutated between paint and
    /// the next hit-test.
    pub selection_range: RCRange, //[i32; 4],
    /// Canvas size at which this frame was built. Stored so `is_still_valid`
    /// can detect a resize without the orchestrator passing size separately.
    pub canvas_size: CanvasSize,
}

impl FrameContext {
    /// Build a per-frame snapshot from the model and canvas size.
    ///
    /// Single-pass construction: frozen prefix sums, visible-region scan, and
    /// scrollable prefix sums are all built in one model-walk per axis instead
    /// of the two separate walks the old `compute_visible_region` +
    /// `compute_pixel_offsets` pair required.
    pub(crate) fn current(model: &dyn CanvasModel, canvas: CanvasSize) -> Self {
        let view = model.get_selected_view();
        let frozen = FrozenRC::from_model(model);

        let frozen_rows = frozen.frozen_rows_count();
        let frozen_cols = frozen.frozen_cols_count();

        // Frozen prefix sums
        // One walk per axis; the totals (last entry) give the Y/X offset where
        // the scrollable band starts, avoiding a redundant sum in the scan below.
        let mut frozen_row_tops = Vec::with_capacity((frozen_rows + 1) as usize);
        let mut frozen_h = 0.0_f64;
        for r in 1..=frozen_rows {
            frozen_row_tops.push(frozen_h);
            frozen_h += row_height(model, r);
        }
        frozen_row_tops.push(frozen_h);

        let mut frozen_col_lefts = Vec::with_capacity((frozen_cols + 1) as usize);
        let mut frozen_w = 0.0_f64;
        for c in 1..=frozen_cols {
            frozen_col_lefts.push(frozen_w);
            frozen_w += col_width(model, c);
        }
        frozen_col_lefts.push(frozen_w);

        let row_first = (frozen_rows + 1).max(view.top_row);
        let col_first = (frozen_cols + 1).max(view.left_column);

        // Scrollable rows: visible extent + prefix-sum in one pass
        // `y` tracks the canvas Y of each row's top edge. When y reaches the
        // canvas bottom we record that row as `row_last` (it may be partially
        // visible), push its trailing entry, and stop. This exactly replicates
        // the semantics of the old two-pass pair.
        let mut row_tops: Vec<f64> = Vec::new();
        let mut row_last = row_first;
        let mut y = HEADER_ROW_HEIGHT + frozen_h;
        let mut acc = 0.0_f64;
        for row in row_first..=LAST_ROW {
            if y >= canvas.h || row == LAST_ROW {
                row_last = row;
                row_tops.push(acc);
                acc += row_height(model, row);
                row_tops.push(acc); // trailing: bottom of row_last
                break;
            }
            row_tops.push(acc);
            let h = row_height(model, row);
            acc += h;
            y += h;
        }

        // Scrollable columns: same merged pattern
        let mut col_lefts: Vec<f64> = Vec::new();
        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_w;
        acc = 0.0;
        for col in col_first..=LAST_COLUMN {
            if x >= canvas.w || col == LAST_COLUMN {
                col_last = col;
                col_lefts.push(acc);
                acc += col_width(model, col);
                col_lefts.push(acc);
                break;
            }
            col_lefts.push(acc);
            let w = col_width(model, col);
            acc += w;
            x += w;
        }

        let vis = VisibleCells {
            first: CellRC {
                row: row_first,
                column: col_first,
            },
            last: CellRC {
                row: row_last,
                column: col_last,
            },
        };
        let offsets = PixelOffsets {
            row_start: row_first,
            row_tops,
            col_start: col_first,
            col_lefts,
            frozen_row_tops,
            frozen_col_lefts,
        };

        FrameContext {
            vis,
            offsets,
            frozen,
            top_row: view.top_row,
            left_column: view.left_column,
            selection_range: view.range,
            canvas_size: canvas,
        }
    }

    /// True when the painted geometry is identical to the current model state.
    ///
    /// Checks scroll origin, frozen band counts, and canvas size — the three
    /// inputs that determine `PixelOffsets`. When all match, the overlay layer
    /// can repaint against this frame without calling `FrameContext::current`.
    pub(crate) fn is_still_valid(&self, model: &dyn CanvasModel, size: CanvasSize) -> bool {
        if size != self.canvas_size {
            return false;
        }
        let view = model.get_selected_view();
        if self.top_row != view.top_row || self.left_column != view.left_column {
            return false;
        }
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        frozen_rows == self.frozen.frozen_rows_count()
            && frozen_cols == self.frozen.frozen_cols_count()
    }

    // Pixel <-> cell mapping  (snapshot-only)
    //
    // Every method here reads exclusively from `self.offsets`,
    // `self.frozen`, `self.vis`, and `self.selection_range`. No model
    // access — what the renderer painted is what gets hit-tested.
    //
    // Off-frame inputs are clamped to the painted region (`pixel_to_*`)
    // or rejected with `None` (`cell_rect`, `autofill_handle`).

    #[inline]
    fn row_in_frame(&self, row: i32) -> bool {
        row <= self.frozen.frozen_rows_count()
            || (row >= self.vis.first.row && row <= self.vis.last.row)
    }

    #[inline]
    fn col_in_frame(&self, col: i32) -> bool {
        col <= self.frozen.frozen_cols_count()
            || (col >= self.vis.first.column && col <= self.vis.last.column)
    }

    /// Width of `col` from the snapshot — frozen-band or visible-band.
    #[inline]
    pub(crate) fn col_extent_at(&self, col: i32) -> f64 {
        if col <= self.frozen.frozen_cols_count() {
            self.offsets.frozen_col_extent(col)
        } else {
            self.offsets.col_extent(col)
        }
    }

    /// Height of `row` from the snapshot.
    #[inline]
    pub(crate) fn row_extent_at(&self, row: i32) -> f64 {
        if row <= self.frozen.frozen_rows_count() {
            self.offsets.frozen_row_extent(row)
        } else {
            self.offsets.row_extent(row)
        }
    }

    /// Left-edge X pixel of `col` at this frame's scroll/freeze.
    /// Caller is expected to gate on `col_in_frame`; off-frame yields the
    /// cumulative-table fallback (`0.0`).
    pub(crate) fn col_to_x(&self, col: i32) -> f64 {
        if col <= self.frozen.frozen_cols_count() {
            HEADER_COL_WIDTH + self.offsets.frozen_col_left(col)
        } else {
            self.frozen.offset.x + self.offsets.col_left(col)
        }
    }

    /// Top-edge Y pixel of `row`.
    pub(crate) fn row_to_y(&self, row: i32) -> f64 {
        if row <= self.frozen.frozen_rows_count() {
            HEADER_ROW_HEIGHT + self.offsets.frozen_row_top(row)
        } else {
            self.frozen.offset.y + self.offsets.row_top(row)
        }
    }

    /// 1-based column at canvas X pixel `x`. Clamps to the painted region
    pub(crate) fn pixel_to_col(&self, x: f64) -> i32 {
        let frozen_cols = self.frozen.frozen_cols_count();
        if x < self.frozen.offset.x {
            let rel = x - HEADER_COL_WIDTH;
            for c in 1..=frozen_cols {
                if rel < self.offsets.frozen_col_lefts[c as usize] {
                    return c;
                }
            }
            return frozen_cols.max(1);
        }
        let rel = x - self.frozen.offset.x;
        let count = (self.vis.last.column - self.vis.first.column + 1) as usize;
        for i in 0..count {
            if rel < self.offsets.col_lefts[i + 1] {
                return self.offsets.col_start + i as i32;
            }
        }
        self.vis.last.column
    }

    /// 1-based row at canvas Y pixel `y`. Clamps to the painted region.
    pub(crate) fn pixel_to_row(&self, y: f64) -> i32 {
        let frozen_rows = self.frozen.frozen_rows_count();
        if y < self.frozen.offset.y {
            let rel = y - HEADER_ROW_HEIGHT;
            for r in 1..=frozen_rows {
                if rel < self.offsets.frozen_row_tops[r as usize] {
                    return r;
                }
            }
            return frozen_rows.max(1);
        }
        let rel = y - self.frozen.offset.y;
        let count = (self.vis.last.row - self.vis.first.row + 1) as usize;
        for i in 0..count {
            if rel < self.offsets.row_tops[i + 1] {
                return self.offsets.row_start + i as i32;
            }
        }
        self.vis.last.row
    }

    /// Pixel rect of `(row, col)` if it falls inside this frame's painted
    /// region (frozen bands + visible scrollable area). Returns `None` for
    /// off-screen cells — the snapshot only describes what was painted.
    pub(crate) fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        if !self.row_in_frame(row) || !self.col_in_frame(col) {
            return None;
        }
        Some(PixelRect {
            top_left: Point {
                x: self.col_to_x(col),
                y: self.row_to_y(row),
            },
            width: self.col_extent_at(col),
            height: self.row_extent_at(row),
        })
    }

    /// Bottom-right pixel of the painted selection — the autofill handle
    /// anchor. `None` for full-row/column/sheet selections (trailing index
    /// at the spreadsheet bound) and for selections whose bottom-right is
    /// off-frame. Reads `selection_range` captured at paint time, so the
    /// handle position is locked to what's on screen even if the model's
    /// selection has since moved.
    pub(crate) fn autofill_handle(&self) -> Option<Point> {
        let r2 = self.selection_range.normalized().r1;
        let c2 = self.selection_range.normalized().c1;
        if r2 >= LAST_ROW || c2 >= LAST_COLUMN {
            return None;
        }
        if !self.row_in_frame(r2) || !self.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: self.col_to_x(c2) + self.col_extent_at(c2),
            y: self.row_to_y(r2) + self.row_extent_at(r2),
        })
    }

    /// Visual rect of the autofill handle — the small square stroked over
    /// the selection's bottom-right corner. Top-left sits exactly at
    /// `autofill_handle()` so the handle pokes outside the selection.
    /// Single source of truth: `draw_selection` paints from this rect and
    /// `hit_test` accepts clicks against an inflated copy of it.
    pub(crate) fn autofill_handle_rect(&self) -> PixelRect {
        if let Some(p) = self.autofill_handle() {
            PixelRect {
                top_left: p,
                width: AUTOFILL_HANDLE_PX,
                height: AUTOFILL_HANDLE_PX,
            }
        } else {
            PixelRect {
                top_left: Point::default(),
                width: AUTOFILL_HANDLE_PX,
                height: AUTOFILL_HANDLE_PX,
            }
        }
    }

    /// Column whose RIGHT edge is within `hit_zone` px of `x`, or `None`.
    pub(crate) fn col_boundary_at(&self, x: f64, hit_zone: f64) -> Option<i32> {
        let frozen_cols = self.frozen.frozen_cols_count();
        for c in 1..=frozen_cols {
            let cur_x = HEADER_COL_WIDTH + self.offsets.frozen_col_lefts[c as usize];
            if (cur_x - x).abs() <= hit_zone {
                return Some(c);
            }
        }
        let count = (self.vis.last.column - self.vis.first.column + 1) as usize;
        for i in 0..count {
            let cur_x = self.frozen.offset.x + self.offsets.col_lefts[i + 1];
            if (cur_x - x).abs() <= hit_zone {
                return Some(self.offsets.col_start + i as i32);
            }
            if cur_x > x + hit_zone {
                break;
            }
        }
        None
    }

    /// Row whose BOTTOM edge is within `hit_zone` px of `y`, or `None`.
    pub(crate) fn row_boundary_at(&self, y: f64, hit_zone: f64) -> Option<i32> {
        let frozen_rows = self.frozen.frozen_rows_count();
        for r in 1..=frozen_rows {
            let cur_y = HEADER_ROW_HEIGHT + self.offsets.frozen_row_tops[r as usize];
            if (cur_y - y).abs() <= hit_zone {
                return Some(r);
            }
        }
        let count = (self.vis.last.row - self.vis.first.row + 1) as usize;
        for i in 0..count {
            let cur_y = self.frozen.offset.y + self.offsets.row_tops[i + 1];
            if (cur_y - y).abs() <= hit_zone {
                return Some(self.offsets.row_start + i as i32);
            }
            if cur_y > y + hit_zone {
                break;
            }
        }
        None
    }

    // Hit-test dispatch

    /// Map `(x, y)` to what the user sees against this frame.
    ///
    /// Negative coordinates return `Outside` (off-canvas). Past the right /
    /// bottom edge the trailing visible cell is returned — the canvas
    /// element's own bounds clip the event before it reaches us in practice.
    pub(crate) fn hit_test(&self, x: f64, y: f64) -> HitTest {
        if x < 0.0 || y < 0.0 {
            return HitTest::Outside;
        }
        if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
            return HitTest::Corner;
        }
        if y < HEADER_ROW_HEIGHT {
            return HitTest::ColHeader(self.pixel_to_col(x));
        }
        if x < HEADER_COL_WIDTH {
            return HitTest::RowHeader(self.pixel_to_row(y));
        }
        let row = self.pixel_to_row(y);
        let column = self.pixel_to_col(x);
        let h = self.autofill_handle_rect();

        let pad = AUTOFILL_HIT_PAD_PX;
        if x >= h.top_left.x - pad
            && x <= h.right() + pad
            && y >= h.top_left.y - pad
            && y <= h.bottom() + pad
        {
            return HitTest::AutofillHandle { row, column };
        }
        HitTest::Cell { row, column }
    }

    /// Probe for a row/column resize handle near `(x, y)`. Dispatched by
    /// header strip — column boundaries are only hit-tested inside the
    /// column-header strip, and vice versa.
    pub(crate) fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        if y < HEADER_ROW_HEIGHT && x > HEADER_COL_WIDTH {
            return self.col_boundary_at(x, tolerance).map(ResizeTarget::Column);
        }
        if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT {
            return self.row_boundary_at(y, tolerance).map(ResizeTarget::Row);
        }
        None
    }
}
