//! Typed dirty signals for `PaintGate`. Replaces the prior `Cell<bool>`
//! payload so `IronCanvas::decide` can dispatch on *what* changed rather
//! than re-deriving intent from `is_still_valid` + `screen_for_blit`. Bit layout
//! mirrors `chrome::pane_region::PaneRegionMask`.

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    #[must_use = "GridSignals encode dirty-bit decisions; dropping the value without acting on it is a paint-skip bug"]
    pub struct GridSignals: u8 {
        /// Reserved bit; no setter raises it today. `screen_for_blit` detects
        /// viewport shifts geometrically.
        const VIEWPORT   = 0b0001;
        const CONTENT    = 0b0010;
        const STRUCTURAL = 0b0100;
        const OVERLAY    = 0b1000;

        const GRID_ANY   = 0b0111;
        const ALL        = 0b1111;
    }
}

impl GridSignals {
    /// `bitflags` v2 spells the zero value `empty()`. Alias kept so call
    /// sites (`Cell::new(GridSignals::EMPTY)`) stay declarative.
    pub const EMPTY: Self = Self::empty();

    pub fn grid_dirty(self) -> bool {
        self.intersects(Self::GRID_ANY)
    }
    pub fn overlay_dirty(self) -> bool {
        self.intersects(Self::OVERLAY)
    }
}
