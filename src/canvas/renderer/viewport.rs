//! Viewport math: which cells are visible, where they sit in pixel space, and
//! how a sheet-coordinate range maps to canvas pixel bounds.
//!
//! All methods in this module are called during a frame and must take `&self`
//! — only `CanvasRenderer::new` and `render` itself mutate `self`.

use ironcalc_base::UserModel;

use crate::canvas::Point;
use crate::coord::CellArea;

use super::super::geometry::{
    col_width, row_height, PixelRect, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
};
use super::super::types::{Axis, FrozenOffset, PixelOffsets, VisibleRegion};
use super::CanvasRenderer;

impl CanvasRenderer {
    /// Map a sheet-coordinate range to canvas pixel bounds, clamping oversized
    /// selections to the canvas edge to avoid O(MAX_COLS) iteration.
    ///
    /// Returns `None` when the range is entirely outside the drawable fold
    /// (scrollable viewport + frozen bands). This moves the visibility
    /// invariant into the type so callers can't accidentally paint garbage
    /// bounds produced by out-of-range offset lookups — e.g. `=BB3` when
    /// column BB is scrolled out of view.
    pub(super) fn range_pixel_bounds(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen: FrozenOffset,
        range: CellArea,
    ) -> Option<PixelRect> {
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);

        // Range entirely past the scrollable fold (right or below) and not
        // anchored in the frozen band → nothing to draw. Guards the
        // out-of-bounds `PixelOffsets` lookups that cause the `=BB3`
        // ghost-row artifact.
        if range.c1 > self.vis.col_last && range.c1 > frozen_cols {
            return None;
        }
        if range.r1 > self.vis.row_last && range.r1 > frozen_rows {
            return None;
        }
        // Range entirely before the scrollable fold (scrolled past on the
        // left/top) and not anchored in the frozen band.
        if range.c2 < self.vis.col_first && range.c2 > frozen_cols {
            return None;
        }
        if range.r2 < self.vis.row_first && range.r2 > frozen_rows {
            return None;
        }

        let x = self.cell_x(model, sheet, range.c1, frozen);
        let y = self.cell_y(model, sheet, range.r1, frozen);
        let right = if range.c2 > self.vis.col_last {
            self.width
        } else {
            self.cell_x(model, sheet, range.c2, frozen) + col_width(model, sheet, range.c2)
        };
        let bottom = if range.r2 > self.vis.row_last {
            self.height
        } else {
            self.cell_y(model, sheet, range.r2, frozen) + row_height(model, sheet, range.r2)
        };
        Some(PixelRect {
            point: Point { x, y },
            width: right - x,
            height: bottom - y,
        })
    }

    fn cell_offset(
        &self,
        model: &UserModel,
        sheet: u32,
        axis: Axis,
        index: i32,
        frozen: FrozenOffset,
    ) -> f64 {
        match axis {
            Axis::Column => {
                let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
                if index <= frozen_cols {
                    return HEADER_COL_WIDTH
                        + 0.5
                        + (1..index).map(|c| col_width(model, sheet, c)).sum::<f64>();
                }
                frozen.x + self.offsets.col_left(index)
            }
            Axis::Row => {
                let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
                if index <= frozen_rows {
                    return HEADER_ROW_HEIGHT
                        + 0.5
                        + (1..index).map(|r| row_height(model, sheet, r)).sum::<f64>();
                }
                frozen.y + self.offsets.row_top(index)
            }
        }
    }

    pub(super) fn cell_x(
        &self,
        model: &UserModel,
        sheet: u32,
        col: i32,
        frozen: FrozenOffset,
    ) -> f64 {
        self.cell_offset(model, sheet, Axis::Column, col, frozen)
    }

    pub(super) fn cell_y(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        frozen: FrozenOffset,
    ) -> f64 {
        self.cell_offset(model, sheet, Axis::Row, row, frozen)
    }

    /// Build a prefix-sum pixel-offset table for all visible rows and columns.
    ///
    /// Each `row_tops[i]` is the cumulative Y distance from `frozen.y` to the
    /// top edge of row `(vis.row_first + i)`. Built in a single O(visible)
    /// pass — same rows/cols that `visible_cells` already iterated. Stored on
    /// `self.offsets` so `cell_x`/`cell_y` become O(1) array lookups instead
    /// of O(visible × R) summations (where R = len of IronCalc's `rows` Vec).
    pub(super) fn build_pixel_offsets(&self, model: &UserModel, sheet: u32) -> PixelOffsets {
        let vis = self.vis;

        let mut row_tops = Vec::with_capacity((vis.row_last - vis.row_first + 2) as usize);
        let mut acc = 0.0_f64;
        for r in vis.row_first..=vis.row_last {
            row_tops.push(acc);
            acc += row_height(model, sheet, r);
        }
        row_tops.push(acc); // one-past-end: bottom edge of last visible row

        let mut col_lefts = Vec::with_capacity((vis.col_last - vis.col_first + 2) as usize);
        acc = 0.0;
        for c in vis.col_first..=vis.col_last {
            col_lefts.push(acc);
            acc += col_width(model, sheet, c);
        }
        col_lefts.push(acc); // one-past-end: right edge of last visible col

        PixelOffsets {
            row_start: vis.row_first,
            row_tops,
            col_start: vis.col_first,
            col_lefts,
        }
    }

    /// Compute the visible (scrollable) cell region.
    ///
    /// This calculation is **completely independent of selection state** to
    /// ensure performance remains constant regardless of selection size
    /// (whole sheet, single cell, etc.). Scans rows/cols until the canvas is
    /// filled, capping at `SCAN_CAP` to prevent O(LAST_ROW) iteration when
    /// many rows are explicitly hidden (height = 0).
    pub(super) fn visible_cells(&self, model: &UserModel) -> VisibleRegion {
        // Conservative cap to prevent runaway iteration in pathological cases.
        // This ensures O(1) performance regardless of sheet size or selection.
        const SCAN_CAP: i32 = 2_048;

        let view = model.get_selected_view();
        let sheet = view.sheet;
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(model, sheet, r)).sum();
        let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(model, sheet, c)).sum();

        let row_first = (frozen_rows + 1).max(view.top_row);
        let col_first = (frozen_cols + 1).max(view.left_column);

        let row_scan_end = (row_first + SCAN_CAP).min(LAST_ROW);
        let mut row_last = row_first;
        let mut y = HEADER_ROW_HEIGHT + frozen_rows_h;
        for row in row_first..=row_scan_end {
            if y >= self.height || row == row_scan_end {
                row_last = row;
                break;
            }
            y += row_height(model, sheet, row);
        }

        let col_scan_end = (col_first + SCAN_CAP).min(LAST_COLUMN);
        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_cols_w;
        for col in col_first..=col_scan_end {
            if x >= self.width || col == col_scan_end {
                col_last = col;
                break;
            }
            x += col_width(model, sheet, col);
        }

        VisibleRegion {
            col_first,
            row_first,
            col_last,
            row_last,
        }
    }
}
