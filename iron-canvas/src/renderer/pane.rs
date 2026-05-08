use crate::{
    geometry::frame::{
        slot::{ColSlot, RowSlot},
        FrameContext,
    },
    renderer::cells::PaneCells,
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

    pub(crate) fn cells<'a>(&'a self, frame: &'a FrameContext) -> PaneCells<'a> {
        PaneCells::new(self, frame)
    }
}
