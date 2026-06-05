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
    ColumnHeader(i32),
    Corner,
    /// Cursor is on the autofill handle. The row/column point at the
    /// drag-target cell (the cell the cursor sits over while the handle
    /// itself protrudes from the selection's bottom-right corner).
    AutofillHandle {
        row: i32,
        column: i32,
    },
    /// Cursor is over a draggable formula-ref overlay. `ref_idx` indexes
    /// into the painted `Vec<FormulaRef>`; `zone` classifies which part of
    /// the rectangle was hit so the host can pick move vs resize behavior.
    /// `grab_row` / `grab_column` are the 1-based cell coordinates under the
    /// pointer at the moment of the hit — Body translation needs them so
    /// the relative cursor position inside the ref is preserved through
    /// the drag.
    FormulaRef {
        ref_idx: usize,
        zone: RefZone,
        grab_row: i32,
        grab_column: i32,
    },
    Outside,
}

/// Cardinal side of a formula-ref rectangle. Used by `RefZone::Edge` to
/// say which edge the cursor is over; drag handlers map this to a single-
/// axis resize direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

/// One of the four corners of a formula-ref rectangle. Used by
/// `RefZone::Corner` to drive two-axis resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Sub-region of a formula-ref rectangle hit by the cursor. Precedence on
/// classification is `Corner` > `Edge` > `Body`: a pointer inside the
/// corner-pad of two intersecting edges reads as a corner, never an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefZone {
    Body,
    Edge(Side),
    Corner(RectCorner),
}

/// A row or column boundary the cursor sits within tolerance of. The index
/// is the row/column whose *trailing edge* the cursor is near — dragging
/// outward enlarges that row/column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeTarget {
    RowEdge(i32),
    ColumnEdge(i32),
}
