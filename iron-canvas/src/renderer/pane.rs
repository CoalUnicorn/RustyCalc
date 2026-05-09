use crate::{
    geometry::frame::{
        slot::{ColSlot, RowSlot},
        FrameContext,
    },
    types::coord::RCRange,
};

/// Identifies one of the four frozen-pane quadrants.
///
/// `PaneRegion::cells(frame)` selects which of the frame's row-slot and
/// col-slot vecs to walk. There is no longer an `origin` field — slot
/// `.left`/`.top` are absolute canvas coordinates.
#[derive(Clone, Copy)]
pub struct PaneRegion {
    pub row_band: Band,
    pub col_band: Band,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Band {
    Frozen,
    Scroll,
}

impl PaneRegion {
    pub(crate) fn top_left() -> Self {
        Self {
            row_band: Band::Frozen,
            col_band: Band::Frozen,
        }
    }
    pub(crate) fn top_right() -> Self {
        Self {
            row_band: Band::Frozen,
            col_band: Band::Scroll,
        }
    }
    pub(crate) fn bottom_left() -> Self {
        Self {
            row_band: Band::Scroll,
            col_band: Band::Frozen,
        }
    }
    pub(crate) fn bottom_right() -> Self {
        Self {
            row_band: Band::Scroll,
            col_band: Band::Scroll,
        }
    }

    pub(crate) fn rows<'a>(&self, frame: &'a FrameContext) -> &'a [RowSlot] {
        match self.row_band {
            Band::Frozen => &frame.frozen_rows,
            Band::Scroll => &frame.scroll_rows,
        }
    }

    pub(crate) fn cols<'a>(&self, frame: &'a FrameContext) -> &'a [ColSlot] {
        match self.col_band {
            Band::Frozen => &frame.frozen_cols,
            Band::Scroll => &frame.scroll_cols,
        }
    }

    /// Address-space rectangle this pane covers. `None` when the pane has no
    /// rows or no cols (a pane is empty whenever the frozen count along that
    /// axis is 0 — e.g. all four panes of an unfrozen sheet are empty except
    /// `bottom_right`).
    ///
    /// The returned range spans `[first_row..=last_row] × [first_col..=last_col]`
    /// from the slot vecs. Hidden rows/cols are NOT removed from the rectangle —
    /// the slot vecs skip hidden lines, but the range stays contiguous so a
    /// dense per-cell buffer keyed by `(row - r1, col - c1)` indexes correctly.
    pub(crate) fn range(&self, frame: &FrameContext) -> Option<RCRange> {
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
