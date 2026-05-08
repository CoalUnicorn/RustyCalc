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
