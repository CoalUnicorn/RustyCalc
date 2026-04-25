//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

use std::ops::RangeInclusive;

use crate::model::{FormulaRef, RCRange, SheetArea};
use crate::renderer::{AutofillTarget, VisibleRegion};
use crate::{CanvasModel, Point};

use super::geometry::{
    col_width, row_height, FrozenRC, PixelRect, HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT,
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
                top_left: Point {
                    x: HEADER_OFFSET,
                    y: along,
                },
                width: HEADER_COL_WIDTH,
                height: thickness,
            },
            Axis::Column => PixelRect {
                top_left: Point {
                    x: along,
                    y: HEADER_OFFSET,
                },
                width: thickness,
                height: HEADER_ROW_HEIGHT,
            },
        }
    }

    /// Extent of the row/column at `index` on `sheet` (row height or column width).
    pub(crate) fn extent(self, model: &dyn CanvasModel, index: i32) -> f64 {
        match self {
            Axis::Row => row_height(model, index),
            Axis::Column => col_width(model, index),
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
    pub(crate) fn visible_band(self, vis: &VisibleRegion) -> RangeInclusive<i32> {
        match self {
            Axis::Row => vis.first.row..=vis.last.row,
            Axis::Column => vis.first.column..=vis.last.column,
        }
    }

    /// Inclusive `(start, end)` of the user's selection along this axis,
    /// read from ironcalc's `SelectedView.range` array laid out as
    /// `[row1, col1, row2, col2]`. Rows live at indices 0/2; columns at 1/3.
    pub(crate) fn selection_range(self, view_range: &[i32; 4]) -> (i32, i32) {
        let (start, end) = match self {
            Axis::Row => (
                view_range[0].min(view_range[2]),
                view_range[0].max(view_range[2]),
            ),
            Axis::Column => (
                view_range[1].min(view_range[3]),
                view_range[1].max(view_range[3]),
            ),
        };
        (start, end)
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
    pub origin: Point,
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
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
        }
    }

    /// Frozen rows x scrollable cols - top-right quadrant.
    pub(crate) fn top_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let rows = frc.row_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_row: *rows.end(),
            rows,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.origin.x,
                y: HEADER_ROW_HEIGHT + HEADER_OFFSET,
            },
            last_col: vis.last.column,
        }
    }

    /// Scrollable rows x frozen cols - bottom-left quadrant.
    pub(crate) fn bottom_left(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        let cols = frc.col_band.clone().unwrap_or(0..=0);
        PaneRegion {
            last_col: *cols.end(),
            rows: vis.first.row..=vis.last.row,
            cols,
            origin: Point {
                x: HEADER_COL_WIDTH + HEADER_OFFSET,
                y: frc.offset.origin.y,
            },
            last_row: vis.last.row,
        }
    }

    /// Scrollable rows x scrollable cols - main area.
    pub(crate) fn bottom_right(frc: &FrozenRC, vis: &VisibleRegion) -> Self {
        PaneRegion {
            rows: vis.first.row..=vis.last.row,
            cols: vis.first.column..=vis.last.column,
            origin: Point {
                x: frc.offset.origin.x,
                y: frc.offset.origin.y,
            },
            last_col: vis.last.column,
            last_row: vis.last.row,
        }
    }
}

/// Overlay ranges passed to `render()` for selection preview drawing.
#[derive(Clone, PartialEq)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<AutofillTarget>,
    pub clipboard: Option<SheetArea>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<RCRange>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_header_rect_pins_x_to_left_strip() {
        let rect = Axis::Row.header_rect(100.0, 20.0);
        assert_eq!(rect.top_left.x, HEADER_OFFSET);
        assert_eq!(rect.top_left.y, 100.0);
        assert_eq!(rect.width, HEADER_COL_WIDTH);
        assert_eq!(rect.height, 20.0);
    }

    #[test]
    fn column_header_rect_pins_y_to_top_strip() {
        let rect = Axis::Column.header_rect(100.0, 20.0);
        assert_eq!(rect.top_left.x, 100.0);
        assert_eq!(rect.top_left.y, HEADER_OFFSET);
        assert_eq!(rect.width, 20.0);
        assert_eq!(rect.height, HEADER_ROW_HEIGHT);
    }

    #[test]
    fn row_header_rect_thickness_maps_to_height() {
        let rect = Axis::Row.header_rect(100.0, 50.0);
        assert_eq!(rect.height, 50.0);
    }

    #[test]
    fn column_header_rect_thickness_maps_to_width() {
        let rect = Axis::Column.header_rect(100.0, 50.0);
        assert_eq!(rect.width, 50.0);
    }
}
