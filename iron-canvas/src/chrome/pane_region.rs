use crate::{
    chrome::Chrome,
    geometry::slot::{ColSlot, RowSlot},
    types::coord::RCRange,
};

/// One of the four frozen-pane quadrants.
///
/// `rows(frame)` / `cols(frame)` select which of the frame's row-slot and
/// col-slot vecs to walk; `range(frame)` returns the address-space `RCRange`
/// they span. Slot `.left` / `.top` are absolute canvas coordinates, so
/// there is no per-pane origin to track.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneRegion {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PaneRegion {
    pub(crate) fn rows<'a>(self, frame: &'a Chrome) -> &'a [RowSlot] {
        match self {
            PaneRegion::TopLeft | PaneRegion::TopRight => &frame.pane_set.frozen_rows,
            PaneRegion::BottomLeft | PaneRegion::BottomRight => &frame.pane_set.scroll_rows,
        }
    }

    pub(crate) fn cols<'a>(self, frame: &'a Chrome) -> &'a [ColSlot] {
        match self {
            PaneRegion::TopLeft | PaneRegion::BottomLeft => &frame.pane_set.frozen_cols,
            PaneRegion::TopRight | PaneRegion::BottomRight => &frame.pane_set.scroll_cols,
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
    pub(crate) fn range(self, frame: &Chrome) -> Option<RCRange> {
        let rows = self.rows(frame);
        let cols = self.cols(frame);
        Some(RCRange {
            r1: rows.first()?.row,
            c1: cols.first()?.col,
            r2: rows.last()?.row,
            c2: cols.last()?.col,
        })
    }
}

/// Bitset over `PaneRegion`. `Chrome.stale_panes` carries one of these
/// to tell `render_grid` which quadrants still need painting. Default
/// after `Chrome::next_frame` is `ALL` (full repaint); Stage 3's
/// `next_frame_with_blit` narrows it when the blit proves cross-axis
/// panes' content is unchanged.
///
/// Bits are ordered to match `PaneRegion as u8`: TopLeft=0, TopRight=1,
/// BottomLeft=2, BottomRight=3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneRegionMask(u8);

impl PaneRegionMask {
    pub(crate) const EMPTY: PaneRegionMask = PaneRegionMask(0);
    pub(crate) const ALL: PaneRegionMask = PaneRegionMask(0b1111);

    pub(crate) fn with(self, region: PaneRegion) -> Self {
        Self(self.0 | (1 << region as u8))
    }

    pub(crate) fn contains(self, region: PaneRegion) -> bool {
        self.0 & (1 << region as u8) != 0
    }

    /// Yields panes in render order (matches the old `render_grid` 4-call
    /// sequence: TopLeft, TopRight, BottomLeft, BottomRight). The order
    /// is load-bearing for `render_grid_blit`'s BottomRight strip-clip
    /// wrapping.
    pub(crate) fn iter(self) -> impl Iterator<Item = PaneRegion> {
        [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ]
        .into_iter()
        .filter(move |&p| self.contains(p))
    }
}
