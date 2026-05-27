//! Slot-Vec recycler. Carries the outgoing frame's allocations forward
//! so steady-state rebuilds only allocate when row/column count outgrows
//! capacity.

use crate::geometry::slot::{ColSlot, RowSlot};

use super::pane_set::PaneSet;

#[derive(Default)]
pub struct RecycledSlots {
    pub frozen_rows: Vec<RowSlot>,
    pub scroll_rows: Vec<RowSlot>,
    pub frozen_cols: Vec<ColSlot>,
    pub scroll_cols: Vec<ColSlot>,
}

impl RecycledSlots {
    pub(super) fn from_pane_set(pane_set: PaneSet) -> Self {
        let PaneSet {
            mut frozen_rows,
            mut scroll_rows,
            mut frozen_cols,
            mut scroll_cols,
            ..
        } = pane_set;
        frozen_rows.clear();
        scroll_rows.clear();
        frozen_cols.clear();
        scroll_cols.clear();
        Self {
            frozen_rows,
            scroll_rows,
            frozen_cols,
            scroll_cols,
        }
    }
}
