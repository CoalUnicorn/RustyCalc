use crate::{
    address::RCRange,
    chrome::Chrome,
    geometry::slot::{AxisSlot, ColSlot, RowSlot},
};

/// One of the four frozen-pane quadrants.
///
/// `rows(frame)` / `cols(frame)` select which of the frame's row-slot and
/// col-slot vecs to walk; `range(frame)` returns the address-space `RCRange`
/// they span. Slot `.left` / `.top` are absolute canvas coordinates, so
/// there is no region-specific origin to track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneRegion {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PaneRegion {
    /// Stable index for arrays stored in TL, TR, BL, BR order.
    pub const fn index(self) -> usize {
        match self {
            PaneRegion::TopLeft => 0,
            PaneRegion::TopRight => 1,
            PaneRegion::BottomLeft => 2,
            PaneRegion::BottomRight => 3,
        }
    }

    pub fn rows(self, frame: &Chrome) -> &[RowSlot] {
        match self {
            PaneRegion::TopLeft | PaneRegion::TopRight => &frame.pane_set.rows.frozen,
            PaneRegion::BottomLeft | PaneRegion::BottomRight => &frame.pane_set.rows.scroll,
        }
    }

    pub fn cols(self, frame: &Chrome) -> &[ColSlot] {
        match self {
            PaneRegion::TopLeft | PaneRegion::BottomLeft => &frame.pane_set.cols.frozen,
            PaneRegion::TopRight | PaneRegion::BottomRight => &frame.pane_set.cols.scroll,
        }
    }

    /// Address-space rectangle this pane covers. `None` when the pane has
    /// no rows or no cols (a pane is empty whenever the frozen count along
    /// that axis is 0 — e.g. all four panes of an unfrozen sheet are
    /// empty except `BottomRight`).
    ///
    /// The returned range spans `[first_row..=last_row] × [first_col..=last_col]`
    /// from the slot vecs. Hidden rows/cols are **not** removed from the
    /// rectangle: the slot vecs carry one slot per id including hidden ones
    /// (hidden slots have zero extent), so the range is contiguous by
    /// construction and a dense per-cell buffer keyed by
    /// `(row - r1, col - c1)` indexes correctly.
    pub fn range(self, frame: &Chrome) -> Option<RCRange> {
        let rows = self.rows(frame);
        let cols = self.cols(frame);
        Some(RCRange {
            r1: rows.first()?.id(),
            c1: cols.first()?.id(),
            r2: rows.last()?.id(),
            c2: cols.last()?.id(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PaneRegion;

    #[test]
    fn pane_region_index_uses_canonical_order() {
        assert_eq!(PaneRegion::TopLeft.index(), 0);
        assert_eq!(PaneRegion::TopRight.index(), 1);
        assert_eq!(PaneRegion::BottomLeft.index(), 2);
        assert_eq!(PaneRegion::BottomRight.index(), 3);
    }
}

/// Structural grid geometry that remains stable across a compatible address
/// shift.
///
/// The frozen counts are not stored: they are exactly the frozen-axis
/// lengths ([`Self::row_lens`]/[`Self::col_lens`]'s first element), which
/// `GridLayout::from_frame` fills from the same `AxisSlots` vecs. Storing
/// both would let equality compare a value against its own source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridShape {
    row_lens: [usize; 2],
    col_lens: [usize; 2],
}

impl GridShape {
    /// Build a shape from the frozen/scroll axis lengths — `[frozen, scroll]`
    /// per axis. Hosts that have to state geometry facts without a live
    /// `Chrome` (diagnostics fixtures, historical snapshot tests) use this;
    /// the invariant that the frozen counts are the first elements holds by
    /// construction, and nothing can mutate a shape afterwards. Dev-only:
    /// every caller constructs a diagnostic fixture, and production shapes
    /// come from `GridLayout::from_frame`.
    #[cfg(feature = "dev-diagnostics")]
    pub const fn from_lens(row_lens: [usize; 2], col_lens: [usize; 2]) -> Self {
        Self { row_lens, col_lens }
    }

    pub const fn row_lens(self) -> [usize; 2] {
        self.row_lens
    }

    pub const fn col_lens(self) -> [usize; 2] {
        self.col_lens
    }

    pub const fn frozen_rows(self) -> i32 {
        self.row_lens[0] as i32
    }

    pub const fn frozen_cols(self) -> i32 {
        self.col_lens[0] as i32
    }
}

/// One dense address rectangle in the piecewise visible grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridSegment {
    region: PaneRegion,
    range: RCRange,
}

impl GridSegment {
    pub const fn region(self) -> PaneRegion {
        self.region
    }

    pub const fn range(self) -> RCRange {
        self.range
    }
}

/// Exact address layout for one frame, stored in TL, TR, BL, BR order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridLayout {
    shape: GridShape,
    segments: [Option<GridSegment>; 4],
}

impl GridLayout {
    pub(super) fn from_frame(frame: &Chrome) -> Self {
        let rows = &frame.pane_set.rows;
        let cols = &frame.pane_set.cols;
        let shape = GridShape {
            row_lens: [rows.frozen.len(), rows.scroll.len()],
            col_lens: [cols.frozen.len(), cols.scroll.len()],
        };
        let segments = [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ]
        .map(|region| {
            region
                .range(frame)
                .map(|range| GridSegment { region, range })
        });

        Self { shape, segments }
    }

    pub const fn shape(self) -> GridShape {
        self.shape
    }

    /// Allocation-free render-order walk over 1–4 dense address segments.
    pub fn segments(&self) -> impl Iterator<Item = GridSegment> + '_ {
        self.segments.iter().copied().flatten()
    }

    /// The segment for `region`, by its canonical TL/TR/BL/BR position.
    /// `from_frame` fills the array in that same order, so the index *is* the
    /// lookup — no scan of the other three entries.
    pub(crate) fn segment(self, region: PaneRegion) -> Option<GridSegment> {
        self.segments[region.index()]
    }
}
