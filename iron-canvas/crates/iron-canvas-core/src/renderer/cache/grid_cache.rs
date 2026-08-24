//! Cross-frame model buffers and painted fingerprint truth for one visible grid.

use std::cell::{Cell, RefCell};

use crate::chrome::{GridLayout, PaneRegion};
use crate::renderer::cell::fingerprint::{FingerprintState, GridLayoutTransition};
use crate::renderer::prepared::FetchedCells;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BufferTruth {
    Valid,
    #[default]
    Stale,
}

/// Dense committed cells for one `GridLayout` segment plus one reusable full-
/// segment fetch bundle. The segment range is owned by `GridCache::layout`.
#[derive(Default)]
pub struct SegmentBuffers {
    cells: FetchedCells,
    prepare_scratch: FetchedCells,
}

impl SegmentBuffers {
    pub(crate) fn take_cells(&mut self) -> FetchedCells {
        std::mem::take(&mut self.cells)
    }

    pub(crate) fn set_cells(&mut self, cells: FetchedCells) {
        self.cells = cells;
    }

    pub(crate) fn take_prepare_scratch(&mut self) -> FetchedCells {
        std::mem::take(&mut self.prepare_scratch)
    }

    pub(crate) fn park_prepare_scratch(&mut self, cells: FetchedCells) {
        self.prepare_scratch = cells;
    }

    #[cfg(feature = "surface-introspection")]
    pub fn preparation_scratch_capacities(&self) -> (usize, usize, usize, usize) {
        self.prepare_scratch.capacities()
    }
}

/// One exact-layout cache. Fixed slots retain TL/TR/BL/BR allocation capacity;
/// `layout` and `buffer_truth` are the semantic validity keys.
pub struct GridCache {
    layout: Cell<Option<GridLayout>>,
    buffers: RefCell<[Option<SegmentBuffers>; 4]>,
    buffer_truth: Cell<BufferTruth>,
    pub(crate) fingerprint: FingerprintState,
}

impl Default for GridCache {
    fn default() -> Self {
        Self {
            layout: Cell::new(None),
            buffers: RefCell::new(std::array::from_fn(|_| None)),
            buffer_truth: Cell::new(BufferTruth::Stale),
            fingerprint: FingerprintState::default(),
        }
    }
}

impl GridCache {
    pub fn layout(&self) -> Option<GridLayout> {
        self.layout.get()
    }

    pub fn buffer_truth(&self) -> BufferTruth {
        self.buffer_truth.get()
    }

    pub(crate) fn classify_layout(&self, candidate: GridLayout) -> GridLayoutTransition {
        self.layout
            .get()
            .map_or(GridLayoutTransition::Incompatible, |committed| {
                GridLayoutTransition::classify(committed, candidate)
            })
    }

    /// Mark model buffers unusable without discarding painted-pixel truth.
    pub fn invalidate_buffers(&self) {
        self.buffer_truth.set(BufferTruth::Stale);
    }

    pub(crate) fn take_prepare_scratch(&self, region: PaneRegion) -> FetchedCells {
        self.buffers.borrow_mut()[region.index()]
            .as_mut()
            .map(SegmentBuffers::take_prepare_scratch)
            .unwrap_or_default()
    }

    pub(crate) fn park_prepare_scratch(&self, region: PaneRegion, cells: FetchedCells) {
        let mut buffers = self.buffers.borrow_mut();
        let slot = &mut buffers[region.index()];
        slot.get_or_insert_with(SegmentBuffers::default)
            .park_prepare_scratch(cells);
    }

    pub(crate) fn take_cells(&self) -> [Option<FetchedCells>; 4] {
        let mut buffers = self.buffers.borrow_mut();
        std::array::from_fn(|index| buffers[index].as_mut().map(SegmentBuffers::take_cells))
    }

    pub(crate) fn replace_cells(&self, layout: GridLayout, mut cells: [Option<FetchedCells>; 4]) {
        let mut buffers = self.buffers.borrow_mut();
        for index in 0..4 {
            match cells[index].take() {
                Some(new_cells) => {
                    let slot = buffers[index].get_or_insert_with(SegmentBuffers::default);
                    let old = slot.take_cells();
                    slot.set_cells(new_cells);
                    slot.park_prepare_scratch(old);
                }
                None => buffers[index] = None,
            }
        }
        self.layout.set(Some(layout));
        self.buffer_truth.set(BufferTruth::Valid);
    }

    pub(crate) fn restore_cells(&self, layout: GridLayout, mut cells: [Option<FetchedCells>; 4]) {
        let mut buffers = self.buffers.borrow_mut();
        for index in 0..4 {
            if let Some(restored) = cells[index].take() {
                buffers[index]
                    .get_or_insert_with(SegmentBuffers::default)
                    .set_cells(restored);
            } else {
                buffers[index] = None;
            }
        }
        self.layout.set(Some(layout));
        self.buffer_truth.set(BufferTruth::Valid);
    }

    /// Commit-only hard reset. Capacity-bearing buffers cease to be semantic
    /// state and the fingerprint pair forgets all painted history.
    pub(crate) fn reset(&self) {
        self.layout.set(None);
        self.buffer_truth.set(BufferTruth::Stale);
        *self.buffers.borrow_mut() = std::array::from_fn(|_| None);
        self.fingerprint.reset();
    }

    #[cfg(feature = "surface-introspection")]
    pub fn preparation_scratch_capacities(&self) -> [(usize, usize, usize, usize); 4] {
        let buffers = self.buffers.borrow();
        std::array::from_fn(|index| {
            buffers[index]
                .as_ref()
                .map(SegmentBuffers::preparation_scratch_capacities)
                .unwrap_or_default()
        })
    }
}
