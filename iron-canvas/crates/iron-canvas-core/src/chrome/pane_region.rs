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
            r1: rows.first()?.row,
            c1: cols.first()?.col,
            r2: rows.last()?.row,
            c2: cols.last()?.col,
        })
    }
}

bitflags::bitflags! {
    /// Bitset over `PaneRegion`. The orchestrator's `GridWork` carries one
    /// of these (or a `BlitPlan` yielding one via `shift_panes()`) and
    /// threads it into `render_grid` as an explicit parameter to tell it
    /// which quadrants still need painting — `Chrome` itself carries no
    /// pane-scope field. Bit positions are pinned to `PaneRegion as u8` so
    /// `with(region)` / `regions()` can map between enum and bit by
    /// left-shift.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PaneRegionMask: u8 {
        const TOP_LEFT     = 1 << PaneRegion::TopLeft as u8;
        const TOP_RIGHT    = 1 << PaneRegion::TopRight as u8;
        const BOTTOM_LEFT  = 1 << PaneRegion::BottomLeft as u8;
        const BOTTOM_RIGHT = 1 << PaneRegion::BottomRight as u8;
    }
}

impl PaneRegionMask {
    /// Aliases for the bitflags-provided `empty()` / `all()` so call sites
    /// (`PaneRegionMask::EMPTY`, `::ALL`) stay declarative.
    pub const EMPTY: Self = Self::empty();
    pub const ALL: Self = Self::all();

    pub fn with(self, region: PaneRegion) -> Self {
        self | Self::from_bits_truncate(1 << region as u8)
    }

    /// Region-typed membership test. Distinct name from `bitflags`'
    /// `contains(other: Self)` so both surfaces stay usable.
    pub fn contains_region(self, region: PaneRegion) -> bool {
        self.bits() & (1 << region as u8) != 0
    }

    /// Yields panes in render order (TopLeft, TopRight, BottomLeft,
    /// BottomRight). Order is load-bearing for `render_grid_blit`'s
    /// BottomRight strip-clip wrapping. Distinct from `bitflags`'
    /// inherent `iter()`, which yields single-bit `Self` values.
    pub fn regions(self) -> impl Iterator<Item = PaneRegion> {
        [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ]
        .into_iter()
        .filter(move |&p| self.contains_region(p))
    }
}
