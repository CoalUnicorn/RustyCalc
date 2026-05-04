//! Renderer-side viewport math: range -> canvas pixel bounds used by overlays.
//!
//! All pixel↔cell math uses the `FrameContext` prefix-sum tables built once
//! per tick in `geometry/frame`. No model access happens here.

use crate::geometry::frame::{FrameContext, VisibleCells};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::types::coord::RCRange;

use super::CanvasRenderer;

impl CanvasRenderer {
    /// Map a sheet-coordinate range to canvas pixel bounds, clamping oversized
    /// selections to the canvas edge.
    ///
    /// Returns `None` when the range is entirely outside the drawable fold
    /// (scrollable viewport + frozen bands). All coordinate math reads from the
    /// `FrameContext` prefix-sum tables — zero model queries.
    pub(super) fn range_pixel_bounds(
        &self,
        frame: &FrameContext,
        range: RCRange,
    ) -> Option<PixelRect> {
        let frozen_rows = frame.frozen.frozen_rows_count();
        let frozen_cols = frame.frozen.frozen_cols_count();

        if !self.range_intersects_fold(&frame.vis, range, frozen_rows, frozen_cols) {
            return None;
        }

        let x = frame.col_to_x(range.c1);
        let y = frame.row_to_y(range.r1);
        let right = if range.c2 > frame.vis.last.column && range.c2 > frozen_cols {
            self.width
        } else {
            frame.col_to_x(range.c2) + frame.col_extent_at(range.c2)
        };
        let bottom = if range.r2 > frame.vis.last.row && range.r2 > frozen_rows {
            self.height
        } else {
            frame.row_to_y(range.r2) + frame.row_extent_at(range.r2)
        };
        Some(PixelRect {
            top_left: Point { x, y },
            width: right - x,
            height: bottom - y,
        })
    }

    /// Does `range` intersect the drawable fold (scrollable viewport plus the
    /// frozen bands)? Guards the prefix-sum lookups against out-of-range refs
    /// like `=BB3` when column BB is off screen.
    fn range_intersects_fold(
        &self,
        vis: &VisibleCells,
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
}
