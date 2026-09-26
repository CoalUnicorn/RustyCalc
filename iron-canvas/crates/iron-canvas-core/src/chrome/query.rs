use crate::RCRange;
use crate::chrome::hit::{HitTest, ResizeTarget};
use crate::geometry::constants::AUTOFILL_HANDLE_PX;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;

use super::Chrome;

impl Chrome {
    pub fn cell_rect(&self, row: i32, col: i32) -> Option<PixelRect> {
        let p = &self.pane_set;
        if !p.row_in_frame(row) || !p.col_in_frame(col) {
            return None;
        }
        Some(PixelRect {
            top_left: Point {
                x: p.col_to_x(col),
                y: p.row_to_y(row),
            },
            width: p.col_extent_at(col),
            height: p.row_extent_at(row),
        })
    }

    /// Map a sheet-coordinate range to canvas pixel bounds, clamping
    /// oversized selections to the canvas edge. `None` when no row or no
    /// column of the range is painted in this frame — the range lies entirely
    /// in the address gap between the frozen band and the scrolled-to band, or
    /// beyond the walked extent. Pure `Chrome` math, no model access.
    ///
    /// Both axes are normalized once, then projected onto their
    /// frozen-plus-scroll union
    /// ([`AxisSlots::project_interval`](crate::geometry::slot::AxisSlots::project_interval)). A range that
    /// starts in the address gap and ends in the scroll band therefore covers
    /// only the ids it names — never the header or frozen pixels between them —
    /// and a range that overlaps only the frozen band still returns its true
    /// rectangle instead of a zero-extent one built from off-frame zeros.
    pub fn range_rect(&self, range: RCRange) -> Option<PixelRect> {
        let norm = range.normalized();
        let p = &self.pane_set;
        let (canvas_w, canvas_h) = self.metrics.logical_extent();

        let (x, mut right) = p.cols.project_interval(norm.c1, norm.c2)?;
        let (y, mut bottom) = p.rows.project_interval(norm.r1, norm.r2)?;

        // The selection continues past the last painted id: there is no slot to
        // take a trailing edge from, so the outline runs to the canvas edge.
        if norm.c2 > p.cols.frozen_count().max(p.cols.last_visible()) {
            right = canvas_w;
        }
        if norm.r2 > p.rows.frozen_count().max(p.rows.last_visible()) {
            bottom = canvas_h;
        }
        Some(PixelRect {
            top_left: Point { x, y },
            // A frozen band taller/wider than the canvas can start past the
            // clamped edge; never hand a painter a negative extent.
            width: (right - x).max(0),
            height: (bottom - y).max(0),
        })
    }

    /// Return the bottom-right corner of the selection's last cell in canvas
    /// pixels. Return `None` if either cell coordinate has no frame slot or
    /// reaches the model's last row or column. A retained edge slot can extend
    /// past the canvas, so the returned point is not clipped to the canvas.
    pub fn autofill_handle(&self, selection_range: RCRange) -> Option<Point> {
        let norm = selection_range.normalized();
        let r2 = norm.r2;
        let c2 = norm.c2;
        let p = &self.pane_set;
        // Selections reaching the grid's last row/column (full-row,
        // full-column, or up against a finite model's data boundary) get
        // no handle — there is nothing beyond to fill into.
        if r2 >= p.rows.last_id || c2 >= p.cols.last_id {
            return None;
        }
        if !p.row_in_frame(r2) || !p.col_in_frame(c2) {
            return None;
        }
        Some(Point {
            x: p.col_to_x(c2) + p.col_extent_at(c2),
            y: p.row_to_y(r2) + p.row_extent_at(r2),
        })
    }

    /// Return the handle's fill rectangle, with [`AUTOFILL_HANDLE_PX`] per side.
    /// Its bottom-right corner is [`autofill_handle`](Chrome::autofill_handle).
    /// The rectangle can extend beyond a cell smaller than the handle.
    /// The selection painter strokes a separate outline around this rectangle.
    /// Return `None` under the same conditions as the anchor.
    pub fn autofill_handle_rect(&self, selection_range: RCRange) -> Option<PixelRect> {
        let p = self.autofill_handle(selection_range)?;
        Some(PixelRect {
            top_left: Point {
                x: p.x - AUTOFILL_HANDLE_PX,
                y: p.y - AUTOFILL_HANDLE_PX,
            },
            width: AUTOFILL_HANDLE_PX,
            height: AUTOFILL_HANDLE_PX,
        })
    }

    pub fn hit_test(&self, x: i32, y: i32) -> HitTest {
        if x < 0 || y < 0 {
            return HitTest::Outside;
        }
        if x < self.cell_origin.x && y < self.cell_origin.y {
            return HitTest::Corner;
        }
        let p = &self.pane_set;
        if y < self.cell_origin.y {
            return match p.cols.pixel_to_id(x) {
                Some(c) => HitTest::ColumnHeader(c),
                None => HitTest::Outside,
            };
        }
        if x < self.cell_origin.x {
            return match p.rows.pixel_to_id(y) {
                Some(r) => HitTest::RowHeader(r),
                None => HitTest::Outside,
            };
        }
        let (Some(row), Some(column)) = (p.rows.pixel_to_id(y), p.cols.pixel_to_id(x)) else {
            return HitTest::Outside;
        };
        // `AutofillHandle` is resolved by `AutofillLayer::hit_test` in
        // the orchestrator's reverse-z walk; the grid path returns plain
        // cell / header / corner / outside.
        HitTest::Cell { row, column }
    }

    pub fn resize_handle_at(&self, x: i32, y: i32, tolerance: i32) -> Option<ResizeTarget> {
        if y < self.col_header_thickness && x > self.row_header_thickness {
            return self
                .pane_set
                .cols
                .boundary_at(x, tolerance)
                .map(ResizeTarget::ColumnEdge);
        }
        if x < self.row_header_thickness && y > self.col_header_thickness {
            return self
                .pane_set
                .rows
                .boundary_at(y, tolerance)
                .map(ResizeTarget::RowEdge);
        }
        None
    }
}
