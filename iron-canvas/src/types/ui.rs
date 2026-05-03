/// What the user sees at a given canvas point, against the last painted frame.
///
/// The active sheet is whatever `IronCanvas` is reflecting at the time of the
/// query, so it is implicit and not encoded into the variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTest {
    Cell {
        row: i32,
        column: i32,
    },
    RowHeader(i32),
    ColHeader(i32),
    Corner,
    /// Cursor is on the autofill handle. Carries the cell under the cursor
    /// because callers always need both — the variant says "begin autofill",
    /// the fields say "drag-target starts here".
    AutofillHandle {
        row: i32,
        column: i32,
    },
    Outside,
}

impl HitTest {
    // Hit-test dispatch

    // Map `(x, y)` to what the user sees against this frame.
    //
    // Negative coordinates return `Outside` (off-canvas). Past the right /
    // bottom edge the trailing visible cell is returned — the canvas
    // element's own bounds clip the event before it reaches us in practice.
    // pub(crate) fn hit_test(&self, x: f64, y: f64) -> HitTest {
    //     if x < 0.0 || y < 0.0 {
    //         return HitTest::Outside;
    //     }
    //     if x < HEADER_COL_WIDTH && y < HEADER_ROW_HEIGHT {
    //         return HitTest::Corner;
    //     }
    //     if y < HEADER_ROW_HEIGHT {
    //         return HitTest::ColHeader(self.pixel_to_col(x));
    //     }
    //     if x < HEADER_COL_WIDTH {
    //         return HitTest::RowHeader(self.pixel_to_row(y));
    //     }
    //     let row = self.pixel_to_row(y);
    //     let column = self.pixel_to_col(x);
    //     let h = self.autofill_handle_rect();

    //     let pad = AUTOFILL_HIT_PAD_PX;
    //     if x >= h.top_left.x - pad
    //         && x <= h.right() + pad
    //         && y >= h.top_left.y - pad
    //         && y <= h.bottom() + pad
    //     {
    //         return HitTest::AutofillHandle { row, column };
    //     }
    //     HitTest::Cell { row, column }
    // }
}
/// A row or column boundary the cursor is currently within tolerance of.
///
/// Returned by `IronCanvas::resize_handle_at` for cursor-style and
/// drag-start decisions. Holds the index of the row/column **whose trailing
/// edge** the cursor is near (i.e. dragging right enlarges that row/column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    Column(i32),
    Row(i32),
}

impl ResizeTarget {
    // Probe for a row/column resize handle near `(x, y)`. Dispatched by
    // header strip — column boundaries are only hit-tested inside the
    // column-header strip, and vice versa.
    // pub(crate) fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
    //     if y < HEADER_ROW_HEIGHT && x > HEADER_COL_WIDTH {
    //         return self.col_boundary_at(x, tolerance).map(ResizeTarget::Column);
    //     }
    //     if x < HEADER_COL_WIDTH && y > HEADER_ROW_HEIGHT {
    //         return self.row_boundary_at(y, tolerance).map(ResizeTarget::Row);
    //     }
    //     None
    // }
}
