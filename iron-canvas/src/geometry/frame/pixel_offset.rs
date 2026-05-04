/// Precomputed pixel offsets for the painted frame.
///
/// Built once per render call alongside `VisibleRegion`. Every geometry
/// query (`col_to_x`, `pixel_to_col`, `cell_rect`, …) reads from here, not
/// from the model — so a hit-test always sees exactly what the renderer
/// painted on this tick.
///
/// Two cumulative tables, each with one trailing entry so deltas yield
/// per-cell extents:
///
/// * `row_tops` / `col_lefts` cover the **scrollable** band, relative to
///   `FrozenRC::offset`. Length = `visible_count + 1`.
/// * `frozen_row_tops` / `frozen_col_lefts` cover the **frozen** band,
///   relative to the header strip (i.e. start at `0.0`, indexed by
///   `frozen_index - 1`). Length = `frozen_count + 1`.
#[derive(Debug, Default)]
pub(crate) struct PixelOffsets {
    pub row_start: i32,
    pub row_tops: Vec<i32>,
    pub col_start: i32,
    pub col_lefts: Vec<i32>,
    pub frozen_row_tops: Vec<i32>,
    pub frozen_col_lefts: Vec<i32>,
}

impl PixelOffsets {
    /// Y distance from `frozen.y` to the top edge of visible-band `row`.
    ///
    /// Returns `0.0` for rows outside the precomputed range.
    #[inline]
    pub fn row_top(&self, row: i32) -> i32 {
        self.row_tops
            .get((row - self.row_start) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// X distance from `frozen.x` to the left edge of visible-band `col`.
    #[inline]
    pub fn col_left(&self, col: i32) -> i32 {
        self.col_lefts
            .get((col - self.col_start) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Y distance from the column-header strip to the top of frozen `row`
    /// (1-based, must be ≤ frozen-rows count). Returns `0.0` for rows
    /// outside the cached range — caller is expected to gate on the frozen
    /// band before calling.
    #[inline]
    pub fn frozen_row_top(&self, row: i32) -> i32 {
        self.frozen_row_tops
            .get((row - 1) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// X distance from the row-header strip to the left of frozen `col`.
    #[inline]
    pub fn frozen_col_left(&self, col: i32) -> i32 {
        self.frozen_col_lefts
            .get((col - 1) as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Height of the visible-band row at `row`, derived from cumulative deltas.
    /// `0.0` if `row` is outside the visible range.
    #[inline]
    pub fn row_extent(&self, row: i32) -> i32 {
        let i = (row - self.row_start) as usize;
        match (self.row_tops.get(i), self.row_tops.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    }

    /// Width of the visible-band column at `col`.
    #[inline]
    pub fn col_extent(&self, col: i32) -> i32 {
        let i = (col - self.col_start) as usize;
        match (self.col_lefts.get(i), self.col_lefts.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    }

    /// Height of the frozen-band row at `row` (1-based).
    #[inline]
    pub fn frozen_row_extent(&self, row: i32) -> i32 {
        let i = (row - 1) as usize;
        match (self.frozen_row_tops.get(i), self.frozen_row_tops.get(i + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    }

    /// Width of the frozen-band column at `col`.
    #[inline]
    pub fn frozen_col_extent(&self, col: i32) -> i32 {
        let i = (col - 1) as usize;
        match (
            self.frozen_col_lefts.get(i),
            self.frozen_col_lefts.get(i + 1),
        ) {
            (Some(a), Some(b)) => b - a,
            _ => 0,
        }
    }
}
