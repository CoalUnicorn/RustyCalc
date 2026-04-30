//! Renderer-side viewport math: pixel coordinates of individual cells, and
//! the sheet-coordinate range → canvas pixel bounds mapping used by every
//! overlay draw.
//!
//! The visible-region scan and pixel-offset prefix-sum table live in
//! `FrameContext` in `geometry.rs`; this module only holds the methods that
//! need access to the renderer's canvas dimensions for clamping (`self.width`,
//! `self.height`).

use crate::model::RCRange;
use crate::{CanvasModel, Point, HEADER_OFFSET};

use super::super::geometry::{
    col_width, row_height, Axis, FrameContext, PixelRect, VisibleRegion, HEADER_COL_WIDTH,
    HEADER_ROW_HEIGHT,
};

use super::CanvasRenderer;

impl CanvasRenderer {
    /// Map a sheet-coordinate range to canvas pixel bounds, clamping oversized
    /// selections to the canvas edge to avoid O(MAX_COLS) iteration.
    ///
    /// Returns `None` when the range is entirely outside the drawable fold
    /// (scrollable viewport + frozen bands). This moves the visibility
    /// invariant into the type so callers can't accidentally paint garbage
    /// bounds produced by out-of-range offset lookups - e.g. `=BB3` when
    /// column BB is scrolled out of view.
    pub(super) fn range_pixel_bounds(
        &self,
        model: &dyn CanvasModel,
        frame: &FrameContext,
        range: RCRange,
    ) -> Option<PixelRect> {
        let frozen_rows = frame.frozen.frozen_rows_count();
        let frozen_cols = frame.frozen.frozen_cols_count();

        if !self.range_intersects_fold(&frame.vis, range, frozen_rows, frozen_cols) {
            return None;
        }

        let x = self.cell_x(model, range.c1, frame);
        let y = self.cell_y(model, range.r1, frame);
        let right = if range.c2 > frame.vis.last.column {
            self.width
        } else {
            self.cell_x(model, range.c2, frame) + col_width(model, range.c2)
        };
        let bottom = if range.r2 > frame.vis.last.row {
            self.height
        } else {
            self.cell_y(model, range.r2, frame) + row_height(model, range.r2)
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
    fn range_intersects_fold(
        &self,
        vis: &VisibleRegion,
        range: RCRange,
        frozen_rows: i32,
        frozen_cols: i32,
    ) -> bool {
        if range.c1 > vis.last.column && range.c1 > frozen_cols {
            return false;
        }
        if range.r1 > vis.last.row && range.r1 > frozen_rows {
            return false;
        }
        if range.c2 < vis.first.column && range.c2 > frozen_cols {
            return false;
        }
        if range.r2 < vis.first.row && range.r2 > frozen_rows {
            return false;
        }
        true
    }

    fn cell_offset(
        &self,
        model: &dyn CanvasModel,
        axis: Axis,
        index: i32,
        frame: &FrameContext,
    ) -> f64 {
        match axis {
            Axis::Column => {
                let frozen_cols = frame.frozen.frozen_cols_count();
                if index <= frozen_cols {
                    return HEADER_COL_WIDTH
                        + HEADER_OFFSET
                        + (1..index).map(|c| col_width(model, c)).sum::<f64>();
                }
                frame.frozen.offset.x + frame.offsets.col_left(index)
            }
            Axis::Row => {
                let frozen_rows = frame.frozen.frozen_rows_count();
                if index <= frozen_rows {
                    return HEADER_ROW_HEIGHT
                        + HEADER_OFFSET
                        + (1..index).map(|r| row_height(model, r)).sum::<f64>();
                }
                frame.frozen.offset.y + frame.offsets.row_top(index)
            }
        }
    }

    pub(super) fn cell_x(&self, model: &dyn CanvasModel, col: i32, frame: &FrameContext) -> f64 {
        self.cell_offset(model, Axis::Column, col, frame)
    }

    pub(super) fn cell_y(&self, model: &dyn CanvasModel, row: i32, frame: &FrameContext) -> f64 {
        self.cell_offset(model, Axis::Row, row, frame)
    }
}
