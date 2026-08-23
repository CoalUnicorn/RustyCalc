use crate::{
    chrome::Chrome,
    geometry::slot::{AxisSlot, ColSlot, RowSlot},
    types::coord::RCRange,
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
    /// from the slot vecs. Hidden rows/cols are NOT removed from the
    /// rectangle — the slot vecs skip hidden lines, but the range stays
    /// contiguous so a dense per-cell buffer keyed by `(row - r1, col - c1)`
    /// indexes correctly.
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

/// Structural grid geometry that remains stable across a compatible address
/// shift.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GridShape {
    row_lens: [usize; 2],
    col_lens: [usize; 2],
    frozen_rows: i32,
    frozen_cols: i32,
}

impl GridShape {
    pub const fn row_lens(self) -> [usize; 2] {
        self.row_lens
    }

    pub const fn col_lens(self) -> [usize; 2] {
        self.col_lens
    }

    pub const fn frozen_rows(self) -> i32 {
        self.frozen_rows
    }

    pub const fn frozen_cols(self) -> i32 {
        self.frozen_cols
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
            frozen_rows: rows.frozen_count(),
            frozen_cols: cols.frozen_count(),
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

    pub(crate) fn segment(self, region: PaneRegion) -> Option<GridSegment> {
        self.segments().find(|segment| segment.region() == region)
    }
}
