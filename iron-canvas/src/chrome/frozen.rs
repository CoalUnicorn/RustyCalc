use crate::geometry::prim::Point;

/// Frozen rows and columns with their pixel origin.
///
/// `rows` / `cols` are counts: today every freeze is anchored at the top-left
/// so `1..=rows` / `1..=cols` is the full extent. A future named-range-anchored
/// freeze would replace these counts with a richer shape. `offset` is the
/// position of the frozen-band separators (sep_x along the col axis, sep_y
/// along the row axis); production builds this in `Chrome::current` Phase E
/// using the dynamic `row_header_thickness`.
#[derive(Debug, Clone, PartialEq)]
pub struct FrozenRC {
    pub rows: i32,
    pub cols: i32,
    pub offset: Point,
}

impl FrozenRC {
    #[inline]
    pub fn frozen_rows_count(&self) -> i32 {
        self.rows
    }

    #[inline]
    pub fn frozen_cols_count(&self) -> i32 {
        self.cols
    }
}
