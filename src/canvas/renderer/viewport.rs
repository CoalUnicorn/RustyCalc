//! Viewport math: which cells are visible, where they sit in pixel space, and
//! how a sheet-coordinate range maps to canvas pixel bounds.
//!
//! All methods in this module are called during a frame and must take `&self`
//! — only `CanvasRenderer::new` and `render` itself mutate `self`.

use ironcalc_base::UserModel;

use crate::canvas::types::CellRC;
use crate::canvas::{Point, HEADER_OFFSET};
use crate::coord::CellArea;

use super::super::geometry::{
    col_width, row_height, FrozenOffset, PixelRect, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
    LAST_COLUMN, LAST_ROW,
};
use super::super::types::{Axis, PixelOffsets, VisibleRegion};
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
        frozen: &FrozenOffset,
        range: CellArea,
    ) -> Option<PixelRect> {
        let sheet = model.get_selected_sheet();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);

        if !self.range_intersects_fold(range, frozen_rows, frozen_cols) {
            return None;
        }

        let x = self.cell_x(model, range.c1, frozen);
        let y = self.cell_y(model, range.r1, frozen);
        let right = if range.c2 > self.vis.last.column {
            self.width
        } else {
            self.cell_x(model, range.c2, frozen) + col_width(model, range.c2)
        };
        let bottom = if range.r2 > self.vis.last.row {
            self.height
        } else {
            self.cell_y(model, range.r2, frozen) + row_height(model, range.r2)
        };
        Some(PixelRect {
            top_left: Point { x, y },
            width: right - x,
            height: bottom - y,
        })
    }

    /// Does `range` intersect the drawable fold (scrollable viewport plus the
    /// frozen bands)? Used by `range_pixel_bounds` to guard the out-of-bounds
    /// `PixelOffsets` lookups that cause the `=BB3` ghost-row artifact.
    ///
    /// A range is drawable when neither corner is strictly past the visible
    /// scrollable band *and* outside the frozen-band anchor on each axis.
    fn range_intersects_fold(&self, range: CellArea, frozen_rows: i32, frozen_cols: i32) -> bool {
        // Range entirely past the scrollable fold (right or below).
        if range.c1 > self.vis.last.column && range.c1 > frozen_cols {
            return false;
        }
        if range.r1 > self.vis.last.row && range.r1 > frozen_rows {
            return false;
        }
        // Range entirely before the scrollable fold (scrolled off to the left/top).
        if range.c2 < self.vis.first.column && range.c2 > frozen_cols {
            return false;
        }
        if range.r2 < self.vis.first.row && range.r2 > frozen_rows {
            return false;
        }
        true
    }

    fn cell_offset(&self, model: &UserModel, axis: Axis, index: i32, frozen: &FrozenOffset) -> f64 {
        let sheet = model.get_selected_sheet();
        match axis {
            Axis::Column => {
                let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
                if index <= frozen_cols {
                    return HEADER_COL_WIDTH
                        + HEADER_OFFSET
                        + (1..index).map(|c| col_width(model, c)).sum::<f64>();
                }
                frozen.origin.x + self.offsets.col_left(index)
            }
            Axis::Row => {
                let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
                if index <= frozen_rows {
                    return HEADER_ROW_HEIGHT
                        + HEADER_OFFSET
                        + (1..index).map(|r| row_height(model, r)).sum::<f64>();
                }
                frozen.origin.y + self.offsets.row_top(index)
            }
        }
    }

    pub(super) fn cell_x(&self, model: &UserModel, col: i32, frozen: &FrozenOffset) -> f64 {
        self.cell_offset(model, Axis::Column, col, frozen)
    }

    pub(super) fn cell_y(&self, model: &UserModel, row: i32, frozen: &FrozenOffset) -> f64 {
        self.cell_offset(model, Axis::Row, row, frozen)
    }

    /// Build a prefix-sum pixel-offset table for all visible rows and columns.
    ///
    /// Each `row_tops[i]` is the cumulative Y distance from `frozen.y` to the
    /// top edge of row `(vis.row_first + i)`. Built in a single O(visible)
    /// pass — same rows/cols that `visible_cells` already iterated. Stored on
    /// `self.offsets` so `cell_x`/`cell_y` become O(1) array lookups instead
    /// of O(visible × R) summations (where R = len of IronCalc's `rows` Vec).
    pub(super) fn build_pixel_offsets(&self, model: &UserModel) -> PixelOffsets {
        let vis = self.vis;

        let mut row_tops = Vec::with_capacity((vis.last.row - vis.first.row + 2) as usize);
        let mut acc = 0.0_f64;
        for r in vis.first.row..=vis.last.row {
            row_tops.push(acc);
            acc += row_height(model, r);
        }
        row_tops.push(acc); // one-past-end: bottom edge of last visible row

        let mut col_lefts = Vec::with_capacity((vis.last.column - vis.first.column + 2) as usize);
        acc = 0.0;
        for c in vis.first.column..=vis.last.column {
            col_lefts.push(acc);
            acc += col_width(model, c);
        }
        col_lefts.push(acc); // one-past-end: right edge of last visible col

        PixelOffsets {
            row_start: vis.first.row,
            row_tops,
            col_start: vis.first.column,
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
        const SCAN_CAP: i32 = 2_048;

        let view = model.get_selected_view();
        let sheet = view.sheet;
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(model, r)).sum();
        let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(model, c)).sum();

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
            y += row_height(model, row);
        }

        let col_scan_end = (col_first + SCAN_CAP).min(LAST_COLUMN);
        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_cols_w;
        for col in col_first..=col_scan_end {
            if x >= self.width || col == col_scan_end {
                col_last = col;
                break;
            }
            x += col_width(model, col);
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
}
