//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

use std::ops::RangeInclusive;

use ironcalc_base::UserModel;

use crate::coord::{CellArea, FormulaRef, SheetArea};
use crate::model::CssColor;

use super::geometry::{
    col_width, row_height, PixelRect, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};

//  Shared axis — row-vs-column symmetry

/// Horizontal vs vertical axis.
///
/// Shared across viewport offset math (`cell_offset` dispatches on axis) and
/// header rect building (`Axis::header_rect`). Carries no payload — the
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
    /// cols); `thickness` is the cell's extent along the same axis (`rh` /
    /// `cw`). The cross-axis extent is always the header strip width/height.
    pub(crate) fn header_rect(self, along: f64, thickness: f64) -> PixelRect {
        match self {
            Axis::Row => PixelRect {
                x: 0.5,
                y: along,
                width: HEADER_COL_WIDTH,
                height: thickness,
            },
            Axis::Column => PixelRect {
                x: along,
                y: 0.5,
                width: thickness,
                height: HEADER_ROW_HEIGHT,
            },
        }
    }
}

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

/// Frozen rows and columns are grouped with their pixel origin
///
/// These and recalculated every frame based on the current model state.
/// The bands use an Option<Range> type. If an axis is None, no freezing occurs.
/// If Some(range), it specifies the indices of the frozen rows or columns.
/// Currently, from_model only supports static freezing from the top (e.g., 1..=N).
/// The Option<Range> structure is intended to support future dynamic freezing,
/// such as scroll-activated rows (named range header)
/// or releasing the freeze when a footer is reached.
///```rust
/// let frc = FrozenRC::from_model(model, sheet);
/// // frc.row_band, frc.col_band - Option<RangeInclusive<i32>>
/// // frc.offset.x/y             - pixel origin of the scrollable area
/// ```
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenRC {
    pub row_band: Option<RangeInclusive<i32>>,
    pub col_band: Option<RangeInclusive<i32>>,
    /// Pixel origin of the scrollable area, computed from the bands above.
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
            row_band: (rows > 0).then_some(1..=rows),
            col_band: (cols > 0).then_some(1..=cols),
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
    pub rows: RangeInclusive<i32>,
    pub cols: RangeInclusive<i32>,
    /// Left edge
    pub start_x: f64,
    /// Top edge
    pub start_y: f64,
    /// Rightmost column that draws its right border.
    pub last_col: i32,
    /// Bottommost row that draws its bottom border.
    pub last_row: i32,
}

impl PaneRegion {
    /// Frozen rows x frozen cols - top-left quadrant.
    pub(crate) fn top_left(frc: &FrozenRC) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_row: *rows.end(),
            last_col: *cols.end(),
            rows,
            cols,
            start_x: HEADER_COL_WIDTH + 0.5,
            start_y: HEADER_ROW_HEIGHT + 0.5,
            last_col: frc.cols,
            last_row: frc.rows,
        }
    }

    /// Frozen rows x scrollable cols - top-right quadrant.
    pub(crate) fn top_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_row: *rows.end(),
            rows,
            cols: vis.col_first..=vis.col_last,
            start_x: frc.offset.x,
            start_y: HEADER_ROW_HEIGHT + 0.5,
            last_col: vis.col_last,
            last_row: frc.rows,
        }
    }

    /// Scrollable rows x frozen cols - bottom-left quadrant.
    pub(crate) fn bottom_left(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_col: *cols.end(),
            rows: vis.row_first..=vis.row_last,
            cols,
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
    pub text_color: CssColor,
    pub underlined: bool,
    pub strike: bool,
    pub lines: Vec<TextLine>,
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
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<CellArea>,
    /// All formula refs extracted from the current formula (multi-color overlays).
    pub formula_refs: Vec<FormulaRef>,
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
    /// Drag overlay changed (autofill preview, point-mode range) - no model change.
    Overlay,
}
