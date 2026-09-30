//! Merge fragment geometry: the model extents a merge needs, and the
//! per-segment pixel fragments derived from them.

use std::collections::HashMap;

use crate::CanvasModel;
use crate::address::RCRange;
use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::slot::AxisSlot;

use super::MergeFragment;

/// Memoized row-height / column-width lookups for one preparation attempt.
///
/// The extent walks are O(height + width) per merge per attempt. A merge
/// larger than the viewport therefore costs one model read per logical row or
/// column. Indexed extent sums are the deferred optimization; a measurement
/// must justify them first.
#[derive(Default)]
pub(super) struct ExtentCache {
    rows: HashMap<i32, i32>,
    cols: HashMap<i32, i32>,
}

impl ExtentCache {
    fn row_extent(&mut self, model: &dyn CanvasModel, sheet: u32, row: i32) -> Option<i32> {
        if let Some(px) = self.rows.get(&row) {
            return Some(*px);
        }
        let px = crate::geometry::slot::row_height(model, sheet, row).extent()?;
        self.rows.insert(row, px);
        Some(px)
    }

    fn col_extent(&mut self, model: &dyn CanvasModel, sheet: u32, col: i32) -> Option<i32> {
        if let Some(px) = self.cols.get(&col) {
            return Some(*px);
        }
        let px = crate::geometry::slot::col_width(model, sheet, col).extent()?;
        self.cols.insert(col, px);
        Some(px)
    }

    /// Sum of the full logical row heights `r1..=r2`, inclusive. An empty
    /// range sums to zero.
    fn row_span(&mut self, model: &dyn CanvasModel, sheet: u32, r1: i32, r2: i32) -> Option<i32> {
        let mut sum = 0i32;
        for row in r1..=r2 {
            sum = sum.checked_add(self.row_extent(model, sheet, row)?)?;
        }
        Some(sum)
    }

    /// Column mirror of [`Self::row_span`].
    fn col_span(&mut self, model: &dyn CanvasModel, sheet: u32, c1: i32, c2: i32) -> Option<i32> {
        let mut sum = 0i32;
        for col in c1..=c2 {
            sum = sum.checked_add(self.col_extent(model, sheet, col)?)?;
        }
        Some(sum)
    }
}

/// Walk `range`'s visible fragments, one per intersecting pane segment, and
/// return them with the primary layout rectangle (the anchor's own segment when
/// it is visible, else the first fragment's).
pub(super) fn merge_geometry(
    model: &dyn CanvasModel,
    frame: &Chrome,
    layout: GridLayout,
    extents: &mut ExtentCache,
    range: RCRange,
) -> Option<(Vec<MergeFragment>, PixelRect)> {
    let sheet = frame.sheet;
    let row_total = extents.row_span(model, sheet, range.r1, range.r2)?;
    let col_total = extents.col_span(model, sheet, range.c1, range.c2)?;
    let mut fragments = Vec::new();
    let mut logical_rect: Option<PixelRect> = None;
    for grid_segment in layout.segments() {
        let Some(cell_range) = intersect(range, grid_segment.range()) else {
            continue;
        };
        let (row, col) = (cell_range.r1, cell_range.c1);
        // `cell_rect` is the in-frame check: the scroll band's address range
        // includes the address gap between the frozen band and the scrolled-to
        // id, and a cell in that gap has no pixels.
        let Some(cell) = frame.cell_rect(row, col) else {
            continue;
        };
        let rows_before = extents.row_span(model, sheet, range.r1, row - 1)?;
        let cols_before = extents.col_span(model, sheet, range.c1, col - 1)?;
        // The segment's own transform: a cell in the frozen band and one in the
        // scroll band resolve the same logical rectangle to different pixels,
        // so each fragment derives its own.
        let own_logical = PixelRect {
            top_left: Point {
                x: cell.left() - cols_before,
                y: cell.top() - rows_before,
            },
            width: col_total,
            height: row_total,
        };
        let Some(area) = segment_cell_area(frame, grid_segment.region()) else {
            continue;
        };
        let Some(rect) = own_logical.intersection(area) else {
            continue;
        };
        // The layout rectangle comes from the anchor's own segment when it is
        // visible; otherwise from the first visible fragment.
        if logical_rect.is_none() || cell_range.contains(range.r1, range.c1) {
            logical_rect = Some(own_logical);
        }
        fragments.push(MergeFragment {
            rect,
            logical_rect: own_logical,
            covered: cell_range,
            sides: [
                rect.left() == own_logical.left(),
                rect.top() == own_logical.top(),
                rect.right() == own_logical.right(),
                rect.bottom() == own_logical.bottom(),
            ],
        });
    }
    let primary = logical_rect.unwrap_or(PixelRect {
        top_left: Point { x: 0, y: 0 },
        width: 0,
        height: 0,
    });
    Some((fragments, primary))
}

/// `rect` moved by the difference between two logical rectangles of one merge
/// (same size, different segment transform).
pub(super) fn translate(origin: PixelRect, target: PixelRect, rect: PixelRect) -> PixelRect {
    PixelRect {
        top_left: Point {
            x: rect.left() + (target.left() - origin.left()),
            y: rect.top() + (target.top() - origin.top()),
        },
        width: rect.width,
        height: rect.height,
    }
}

/// Inclusive intersection of two ranges, or `None` when they are disjoint.
pub(super) fn intersect(a: RCRange, b: RCRange) -> Option<RCRange> {
    let a = a.normalized();
    let b = b.normalized();
    let r1 = a.r1.max(b.r1);
    let c1 = a.c1.max(b.c1);
    let r2 = a.r2.min(b.r2);
    let c2 = a.c2.min(b.c2);
    (r1 <= r2 && c1 <= c2).then_some(RCRange { r1, c1, r2, c2 })
}

/// Pixel rectangle covered by one pane segment's visible slots.
pub(super) fn segment_cell_area(frame: &Chrome, region: PaneRegion) -> Option<PixelRect> {
    let rows = region.rows(frame);
    let cols = region.cols(frame);
    let first_row = rows.first()?;
    let last_row = rows.last()?;
    let first_col = cols.first()?;
    let last_col = cols.last()?;
    Some(PixelRect {
        top_left: Point {
            x: first_col.start(),
            y: first_row.start(),
        },
        width: last_col.end() - first_col.start(),
        height: last_row.end() - first_row.start(),
    })
}
