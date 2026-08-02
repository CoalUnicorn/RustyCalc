//! Slot-Vec recycler. Carries the outgoing frame's allocations forward
//! so steady-state rebuilds only allocate when row/column count outgrows
//! capacity.
//!
//! Stage 4: `Orchestrator` owns one standing `RecycledSlots` (`spare_slots`)
//! across paint attempts, rather than a fresh one being derived from `prev`
//! inline on every `FramePath::Fresh` build. `Orchestrator::paint_fresh_regime`
//! takes the pool's vectors to build the candidate — leaving the committed
//! `prev` untouched for the duration of that build — and only folds `prev`'s
//! own outgoing vectors back into the pool once the candidate is confirmed
//! good, via [`Self::from_pane_set`]. `pub(crate)` (not `pub(super)`) so that
//! cross-module commit step is reachable from `orchestrator.rs`.

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
    /// Drain `pane_set`'s four slot Vecs (`.clear()`, keeping their heap
    /// capacity) into a fresh recycling pool. Generic over *which* `Chrome`
    /// the caller is recycling — an outgoing committed frame (the ordinary
    /// per-attempt case) or an uncommitted, held candidate (once a future
    /// stage wires that decision) both drain the same way.
    pub(crate) fn from_pane_set(pane_set: PaneSet) -> Self {
        let PaneSet { rows, cols, .. } = pane_set;
        let mut frozen_rows = rows.frozen;
        let mut scroll_rows = rows.scroll;
        let mut frozen_cols = cols.frozen;
        let mut scroll_cols = cols.scroll;
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
