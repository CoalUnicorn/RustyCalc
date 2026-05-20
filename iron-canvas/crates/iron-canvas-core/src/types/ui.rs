/// What the cursor is over at a canvas point, resolved against the last
/// painted frame. Sheet is implicit (whichever sheet `IronCanvas` is
/// currently reflecting).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTest {
    Cell {
        row: i32,
        column: i32,
    },
    RowHeader(i32),
    ColHeader(i32),
    Corner,
    /// Cursor is on the autofill handle. The row/column point at the
    /// drag-target cell (the cell the cursor sits over while the handle
    /// itself protrudes from the selection's bottom-right corner).
    AutofillHandle {
        row: i32,
        column: i32,
    },
    Outside,
}

/// A row or column boundary the cursor sits within tolerance of. The index
/// is the row/column whose *trailing edge* the cursor is near — dragging
/// outward enlarges that row/column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    Column(i32),
    Row(i32),
}
