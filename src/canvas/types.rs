//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

use std::ops::RangeInclusive;

use ironcalc_base::UserModel;

use super::geometry::{
    col_width, row_height, PixelRect, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};

//  Frozen-pane geometry

/// Pixel origin of the scrollable (non-frozen) grid area.
///
/// Passed to coordinate helpers and drawing functions so call sites read:
/// `cell_x(model, sheet, col, frozen)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrozenOffset {
    /// X pixel where the scrollable column area begins.
    pub x: f64,
    /// Y pixel where the scrollable row area begins.
    pub y: f64,
}

/// Frozen row/col counts together with their pixel origin.
///
/// Built once per frame from the model. Replaces the five scattered locals
/// (`frozen_rows`, `frozen_cols`, `frozen_rows_h`, `frozen_cols_w`, `frozen`)
/// that previously littered the top of `render()`.
///
/// ```text
/// let frc = FrozenRC::from_model(model, sheet);
/// // frc.rows, frc.cols - count of frozen rows/cols
/// // frc.offset.x/y    - pixel origin of the scrollable area
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FrozenRC {
    pub rows: i32,
    pub cols: i32,
    /// Pixel origin of the scrollable area, computed from the counts above.
    pub offset: FrozenOffset,
}

impl FrozenRC {
    /// Read frozen geometry from the model for `sheet`.
    pub fn from_model(model: &UserModel, sheet: u32) -> Self {
        let rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let h: f64 = (1..=rows).map(|r| row_height(model, sheet, r)).sum();
        let w: f64 = (1..=cols).map(|c| col_width(model, sheet, c)).sum();
        FrozenRC {
            rows,
            cols,
            offset: FrozenOffset {
                x: HEADER_COL_WIDTH + w + if cols > 0 { FROZEN_SEP } else { 0.0 },
                y: HEADER_ROW_HEIGHT + h + if rows > 0 { FROZEN_SEP } else { 0.0 },
            },
        }
    }
}

//  Pane rendering

/// Describes one of the four frozen-pane quadrants for `render_pane`.
///
/// Build with a named constructor so the quadrant name appears at the call site:
/// ```text
/// render_pane(model, sheet, &mut texts, PaneRegion::top_left(&frc));
/// render_pane(model, sheet, &mut texts, PaneRegion::bottom_right(&frc, &vis));
/// ```
#[derive(Clone)]
pub(crate) struct PaneRegion {
    /// Row range to render (inclusive).
    pub rows: RangeInclusive<i32>,
    /// Column range to render (inclusive).
    pub cols: RangeInclusive<i32>,
    /// Left edge pixel of this pane.
    pub start_x: f64,
    /// Top edge pixel of this pane.
    pub start_y: f64,
    /// Rightmost column that draws its right border.
    pub last_col: i32,
    /// Bottommost row that draws its bottom border.
    pub last_row: i32,
}

impl PaneRegion {
    /// Frozen rows x frozen cols - top-left quadrant.
    pub(crate) fn top_left(frc: &FrozenRC) -> Self {
        PaneRegion {
            rows: 1..=frc.rows,
            cols: 1..=frc.cols,
            start_x: HEADER_COL_WIDTH + 0.5,
            start_y: HEADER_ROW_HEIGHT + 0.5,
            last_col: frc.cols,
            last_row: frc.rows,
        }
    }

    /// Frozen rows x scrollable cols - top-right quadrant.
    pub(crate) fn top_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: 1..=frc.rows,
            cols: vis.col_first..=vis.col_last,
            start_x: frc.offset.x,
            start_y: HEADER_ROW_HEIGHT + 0.5,
            last_col: vis.col_last,
            last_row: frc.rows,
        }
    }

    /// Scrollable rows x frozen cols - bottom-left quadrant.
    pub(crate) fn bottom_left(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: vis.row_first..=vis.row_last,
            cols: 1..=frc.cols,
            start_x: HEADER_COL_WIDTH + 0.5,
            start_y: frc.offset.y,
            last_col: frc.cols,
            last_row: vis.row_last,
        }
    }

    /// Scrollable rows x scrollable cols - main area.
    pub(crate) fn bottom_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: vis.row_first..=vis.row_last,
            cols: vis.col_first..=vis.col_last,
            start_x: frc.offset.x,
            start_y: frc.offset.y,
            last_col: vis.col_last,
            last_row: vis.row_last,
        }
    }
}

// Pre-computed text layout

/// One visual line of text inside a cell, positioned for center-aligned rendering.
pub(crate) struct TextLine {
    pub text: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
}

/// Pre-computed text layout for one cell.
///
/// Collected during Phase 1 (cell backgrounds) and painted in Phase 4 so
/// text always renders on top of selection fills and header lines.
pub(crate) struct CellText {
    /// Clip rectangle - the cell's pixel bounds.
    pub clip: PixelRect,
    pub font: String,
    pub font_size_px: f64,
    pub text_color: String,
    pub underlined: bool,
    pub strike: bool,
    pub lines: Vec<TextLine>,
}

// Cell rendering params

/// Row/column rectangle in sheet coordinates (always normalised: min <= max).
pub(crate) struct SheetRange {
    pub row_min: i32,
    pub col_min: i32,
    pub row_max: i32,
    pub col_max: i32,
}

impl SheetRange {
    /// Normalised bounding box of a clipboard copy range.
    pub(crate) fn from_clipboard(cb: &ClipboardRange) -> Self {
        SheetRange {
            row_min: cb.r1.min(cb.r2),
            col_min: cb.c1.min(cb.c2),
            row_max: cb.r1.max(cb.r2),
            col_max: cb.c1.max(cb.c2),
        }
    }

    /// Normalised bounding box of a point-mode formula range.
    pub(crate) fn from_point_range(pr: &SheetRect) -> Self {
        SheetRange {
            row_min: pr.r1.min(pr.r2),
            col_min: pr.c1.min(pr.c2),
            row_max: pr.r1.max(pr.r2),
            col_max: pr.c1.max(pr.c2),
        }
    }

    /// Selection range expanded to include the autofill drag target.
    ///
    /// Takes `view.range` (a `[i32; 4]` from IronCalc) and grows it to cover
    /// `target` - the cell the user is dragging the autofill handle toward.
    pub(crate) fn from_autofill_extend(range: [i32; 4], target: AutofillTarget) -> Self {
        let [r1, c1, r2, c2] = range;
        SheetRange {
            row_min: r1.min(r2).min(target.row),
            col_min: c1.min(c2).min(target.col),
            row_max: r1.max(r2).max(target.row),
            col_max: c1.max(c2).max(target.col),
        }
    }
}

/// Pixel-space bounding box returned by `range_pixel_bounds`.
///
/// Prefer `.width()` / `.height()` over manual `x2 - x1` arithmetic.
pub(crate) struct PixelBounds {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

impl PixelBounds {
    pub fn width(&self) -> f64 {
        self.x2 - self.x1
    }
    pub fn height(&self) -> f64 {
        self.y2 - self.y1
    }
}

/// The four index boundaries of the visible (scrollable) area.
#[derive(Copy, Clone, Default)]
pub(crate) struct VisibleRegion {
    /// First scrollable column on screen.
    pub col_first: i32,
    /// First scrollable row on screen.
    pub row_first: i32,
    /// Last scrollable column on screen.
    pub col_last: i32,
    /// Last scrollable row on screen.
    pub row_last: i32,
}

/// Precomputed pixel offsets for visible rows and columns.
///
/// Built once per render call from the same iteration used to determine
/// `VisibleRegion`. Eliminates the O(visible_range x R) summation inside
/// `cell_x`/`cell_y` - each lookup becomes O(1).
///
/// Offsets are relative to `FrozenOffset`: `row_tops[i]` is the Y distance
/// from `frozen.y` to the top edge of row `(row_start + i as i32)`.
/// `row_start` equals `vis.row_first`.
#[derive(Default)]
pub(crate) struct PixelOffsets {
    pub row_start: i32,
    /// `row_tops[i]` = cumulative Y from `frozen.y` to top of row `(row_start + i)`.
    pub row_tops: Vec<f64>,
    pub col_start: i32,
    /// `col_lefts[i]` = cumulative X from `frozen.x` to left of col `(col_start + i)`.
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

/// Which outer edges of a cell rect should receive a border stroke.
///
/// Passed to `render_cell_style` so the intent is clear at every call site
/// instead of two anonymous `bool` arguments.
#[derive(Copy, Clone)]
pub(crate) struct CellEdges {
    pub right: bool,
    pub bottom: bool,
}

/// Controls whether `draw_dashed_range` fills the interior with a light tint.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum DashFill {
    /// Outline only (used for clipboard marching ants).
    Outline,
    /// Outline + semi-transparent fill tint (used for point-mode range).
    Tinted,
}

//  Public overlay types (used by worksheet.rs)

/// The target cell during an autofill-handle drag.
///
/// Replaces the anonymous `Option<(i32, i32)>` in `RenderOverlays` with a
/// named struct so the fields are self-documenting at every call site.
#[derive(Copy, Clone, PartialEq)]
pub struct AutofillTarget {
    pub row: i32,
    pub col: i32,
}

/// Overlay ranges passed to `render()` for selection preview drawing.
#[derive(Clone, PartialEq)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    /// Last Ctrl+C copied range: `(sheet, r1, c1, r2, c2)`.
    pub clipboard: Option<ClipboardRange>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<SheetRect>,
}

/// A copied range on a specific sheet.
#[derive(Copy, Clone, PartialEq)]
pub struct ClipboardRange {
    pub sheet: u32,
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

/// A rectangular region of cells (no sheet - always the current sheet).
///
/// Fields are raw and potentially un-normalised: `r1` may be greater than `r2`
/// during a drag. Use [`SheetRange`] (renderer-internal) when you need a
/// normalised form, or call [`SheetRect::extend_trailing`] to produce a new
/// `SheetRect` with the trailing corner moved by one arrow step.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct SheetRect {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

impl SheetRect {
    /// Construct a 1x1 rect anchored at a single cell.
    pub fn from_cell(row: i32, col: i32) -> Self {
        SheetRect {
            r1: row,
            c1: col,
            r2: row,
            c2: col,
        }
    }

    /// Return a new rect with the trailing corner (r2, c2) moved one step in
    /// the arrow-key direction. The anchor (r1, c1) is preserved. Clamps at 1.
    pub fn extend_trailing(self, key: &str) -> Self {
        let (r2, c2) = match key {
            "ArrowDown" => (self.r2 + 1, self.c2),
            "ArrowUp" => ((self.r2 - 1).max(1), self.c2),
            "ArrowLeft" => (self.r2, (self.c2 - 1).max(1)),
            "ArrowRight" => (self.r2, self.c2 + 1),
            _ => (self.r2, self.c2),
        };
        SheetRect {
            r1: self.r1,
            c1: self.c1,
            r2,
            c2,
        }
    }
}

/// Hint to the canvas renderer about the minimum work needed for this repaint.
///
/// Currently `CanvasRenderer::render` treats all modes identically.
/// The enum is in place so future optimisations (skip layout recalc for
/// `FormatOnly`, skip cell-text for `ViewportUpdate`) can be added
/// without another architectural change.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CanvasRenderMode {
    /// Content or structure changed - repaint all cells (default).
    #[default]
    Full,
    /// Only formatting changed - repaint without model recalculation.
    FormatOnly,
    /// Navigation only - update selection box and scroll position.
    ViewportUpdate,
}
