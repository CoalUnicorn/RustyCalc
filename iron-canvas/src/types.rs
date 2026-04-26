//! Canvas domain types - the authoritative type definitions for the canvas module.
//!
//! Types are split by visibility:
//! - `pub(crate)` - renderer-internal: text layout, pane geometry, drawing params
//! - `pub` - worksheet-visible: overlay state passed in from the Leptos component

use std::ops::RangeInclusive;

use crate::model::{CellAddress, FormulaRef, RCRange, SheetArea};
use crate::renderer::{AutofillTarget, VisibleRegion};
use crate::{CanvasModel, Point};

use super::geometry::{
    col_width, row_height, CanvasSize, FrozenRC, PixelRect, HEADER_COL_WIDTH, HEADER_OFFSET,
    HEADER_ROW_HEIGHT,
};

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

/// Outer edge of a cell rect that may be forced to stroke a border because
/// the cell sits on a pane boundary. Only `Right` and `Bottom` are valid -
/// left/top are inner edges resolved against neighbour cells inside the
/// pane.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum OuterEdge {
    Right,
    Bottom,
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

    /// Outer borders this `(row, col)` must draw because it sits on a pane
    /// boundary. Empty slice for interior cells. Static slices - no
    /// allocation per cell.
    pub(crate) fn outer_edges_at(&self, row: i32, col: i32) -> &'static [OuterEdge] {
        match (col == self.last_col, row == self.last_row) {
            (true, true) => &[OuterEdge::Right, OuterEdge::Bottom],
            (true, false) => &[OuterEdge::Right],
            (false, true) => &[OuterEdge::Bottom],
            (false, false) => &[],
        }
    }

    /// Walk every visible cell in this pane, yielding pixel rect + outer
    /// edges per cell. Replaces the open-coded row/col iteration that used
    /// to live in `render_pane` / `render_pane_row`. The caller passes the
    /// canvas size so the walker can early-break past the canvas edge.
    pub(crate) fn cells<'a>(
        &'a self,
        model: &'a dyn CanvasModel,
        canvas: CanvasSize,
    ) -> PaneCells<'a> {
        let column_widths: Vec<(i32, f64)> = self
            .cols
            .clone()
            .map(|c| (c, col_width(model, c)))
            .collect();
        PaneCells {
            pane: self,
            model,
            sheet: model.get_selected_sheet(),
            canvas,
            column_widths,
            row_iter: self.rows.clone(),
            row_top: self.origin.y,
            current_row: None,
            col_idx: 0,
            col_left: self.origin.x,
        }
    }
}

/// One cell yielded by a `PaneCells` walk: the address, its pixel rect at
/// the current scroll, and any outer pane-boundary borders the renderer
/// must force-draw on it.
#[derive(Clone, Copy)]
pub(crate) struct CellSlot {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub outer_edges: &'static [OuterEdge],
}

/// Stateful walk over the cells of a `PaneRegion`. Caches per-pane column
/// widths once, threads a row-top accumulator across rows, and skips
/// hidden rows/columns as well as cells that fall off the canvas. Replaces
/// the parameter cluster that used to feed `render_pane_row`.
pub(crate) struct PaneCells<'a> {
    pane: &'a PaneRegion,
    model: &'a dyn CanvasModel,
    sheet: u32,
    canvas: CanvasSize,
    column_widths: Vec<(i32, f64)>,
    row_iter: RangeInclusive<i32>,
    row_top: f64,
    /// `(row, height)` of the row currently being walked. `None` when we
    /// need to pull the next row from `row_iter`.
    current_row: Option<(i32, f64)>,
    col_idx: usize,
    col_left: f64,
}

impl<'a> Iterator for PaneCells<'a> {
    type Item = CellSlot;

    fn next(&mut self) -> Option<CellSlot> {
        loop {
            // Acquire a row strip if we don't have one in flight.
            if self.current_row.is_none() {
                let row = self.row_iter.next()?;
                // Past the canvas bottom - nothing more in this pane will
                // ever be visible.
                if self.row_top >= self.canvas.h {
                    return None;
                }
                let h = row_height(self.model, row);
                // Hidden row (height 0): skip without advancing row_top.
                if h <= 0.0 {
                    continue;
                }
                self.current_row = Some((row, h));
                self.col_idx = 0;
                self.col_left = self.pane.origin.x;
            }
            let (row, row_h) = self.current_row.expect("set above");

            while self.col_idx < self.column_widths.len() {
                let (col, col_w) = self.column_widths[self.col_idx];
                let col_left = self.col_left;
                self.col_idx += 1;
                self.col_left += col_w;

                if col_left >= self.canvas.w {
                    break;
                }
                if col_w <= 0.0 {
                    continue;
                }
                let rect = PixelRect {
                    top_left: Point {
                        x: col_left,
                        y: self.row_top,
                    },
                    width: col_w,
                    height: row_h,
                };
                if !rect.intersects(self.canvas) {
                    continue;
                }
                return Some(CellSlot {
                    addr: CellAddress {
                        sheet: self.sheet,
                        row,
                        column: col,
                    },
                    rect,
                    outer_edges: self.pane.outer_edges_at(row, col),
                });
            }

            self.row_top += row_h;
            self.current_row = None;
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

    #[test]
    fn row_strip_start_is_below_top_header() {
        assert_eq!(Axis::Row.strip_start(), HEADER_ROW_HEIGHT + HEADER_OFFSET);
    }

    #[test]
    fn column_strip_start_is_right_of_left_header() {
        assert_eq!(Axis::Column.strip_start(), HEADER_COL_WIDTH + HEADER_OFFSET);
    }

    fn vis(rows: (i32, i32), cols: (i32, i32)) -> crate::renderer::VisibleRegion {
        crate::renderer::VisibleRegion {
            first: crate::geometry::CellRC {
                row: rows.0,
                column: cols.0,
            },
            last: crate::geometry::CellRC {
                row: rows.1,
                column: cols.1,
            },
        }
    }

    #[test]
    fn row_visible_band_uses_first_last_row() {
        let v = vis((3, 17), (5, 12));
        let band = Axis::Row.visible_band(&v);
        assert_eq!(*band.start(), 3);
        assert_eq!(*band.end(), 17);
    }

    #[test]
    fn column_visible_band_uses_first_last_column() {
        let v = vis((3, 17), (5, 12));
        let band = Axis::Column.visible_band(&v);
        assert_eq!(*band.start(), 5);
        assert_eq!(*band.end(), 12);
    }

    fn frozen(rows: Option<(i32, i32)>, cols: Option<(i32, i32)>, origin: Point) -> FrozenRC {
        FrozenRC {
            row_band: rows.map(|(s, e)| s..=e),
            col_band: cols.map(|(s, e)| s..=e),
            offset: crate::geometry::FrozenOffset { origin },
        }
    }

    #[test]
    fn pane_top_left_origin_is_pinned_to_header_corner() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let p = PaneRegion::top_left(&frc);
        assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
        assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
        assert_eq!(p.last_row, 2);
        assert_eq!(p.last_col, 3);
    }

    #[test]
    fn pane_top_right_origin_uses_frozen_x_and_header_y() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::top_right(&frc, &v);
        assert_eq!(p.origin.x, 200.0);
        assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
        assert_eq!(*p.cols.start(), 4);
        assert_eq!(p.last_col, 11);
    }

    #[test]
    fn pane_bottom_left_origin_uses_header_x_and_frozen_y() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::bottom_left(&frc, &v);
        assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
        assert_eq!(p.origin.y, 100.0);
        assert_eq!(*p.rows.start(), 3);
        assert_eq!(p.last_row, 9);
    }

    #[test]
    fn pane_bottom_right_origin_matches_frozen_offset() {
        let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
        let v = vis((3, 9), (4, 11));
        let p = PaneRegion::bottom_right(&frc, &v);
        assert_eq!(p.origin.x, 200.0);
        assert_eq!(p.origin.y, 100.0);
        assert_eq!(p.last_row, 9);
        assert_eq!(p.last_col, 11);
    }
}
