//! Typed dirty signals for `PaintGate`. Replaces the prior `Cell<bool>`
//! payload so `IronCanvas::decide` can dispatch on *what* changed rather
//! than re-deriving intent from `is_still_valid` + `try_blit`. Bit layout
//! mirrors `chrome::pane_region::PaneRegionMask`.

use std::ops::{BitOr, BitOrAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GridSignals(u8);

impl GridSignals {
    pub(crate) const EMPTY: Self = Self(0);
    /// Reserved slot. No setter raises it today — geometric `try_blit`
    /// detects viewport shifts directly. Kept for a future typed
    /// scroll-changed setter (option A from the blit-fix menu).
    #[allow(dead_code)]
    pub(crate) const VIEWPORT: Self = Self(0b0001);
    pub(crate) const CONTENT: Self = Self(0b0010);
    pub(crate) const STRUCTURAL: Self = Self(0b0100);
    pub(crate) const OVERLAY: Self = Self(0b1000);

    pub(crate) const GRID_ANY: Self = Self(0b0111);
    #[allow(dead_code)]
    pub(crate) const ALL: Self = Self(0b1111);

    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }
    pub(crate) fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub(crate) fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    pub(crate) fn grid_dirty(self) -> bool {
        self.intersects(Self::GRID_ANY)
    }
    pub(crate) fn overlay_dirty(self) -> bool {
        self.intersects(Self::OVERLAY)
    }
}

impl BitOr for GridSignals {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for GridSignals {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
