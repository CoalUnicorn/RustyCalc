//! Typed dirty signals for `PaintGate`. Replaces the prior `Cell<bool>`
//! payload so `IronCanvas::decide` can dispatch on *what* changed rather
//! than re-deriving intent from `is_still_valid` + `try_blit`. Bit layout
//! mirrors `chrome::pane_region::PaneRegionMask`.

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct GridSignals: u8 {
        /// Reserved slot. No setter raises it today — geometric `try_blit`
        /// detects viewport shifts directly. Kept for a future typed
        /// scroll-changed setter (option A from the blit-fix menu).
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
    pub(crate) const EMPTY: Self = Self::empty();

    pub(crate) fn grid_dirty(self) -> bool {
        self.intersects(Self::GRID_ANY)
    }
    pub(crate) fn overlay_dirty(self) -> bool {
        self.intersects(Self::OVERLAY)
    }
}
