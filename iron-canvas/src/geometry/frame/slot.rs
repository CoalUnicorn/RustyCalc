//! Per-axis visible slots on the painted frame.
//!
//! A `RowSlot` / `ColSlot` is the axis-level peer of `CellSlot`: the index
//! plus the absolute canvas coordinate of its leading edge plus its extent.
//! `FrameContext` stores four vecs of these (frozen / scrollable × row /
//! column) and every pixel↔cell query reads them — no prefix-sum decoding.

#[derive(Clone, Copy, Debug)]
pub struct RowSlot {
    pub row: i32,
    /// Absolute canvas Y of this row's top edge.
    pub top: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColSlot {
    pub col: i32,
    /// Absolute canvas X of this column's left edge.
    pub left: i32,
    pub width: i32,
}

#[derive(Clone, Debug)]
pub struct PaneColumns {
    pub frozen: Vec<ColSlot>,
    pub scroll: Vec<ColSlot>,
    pub frozen_offset_x: i32,
}

#[derive(Clone, Debug)]
pub struct PaneRows {
    pub frozen: Vec<RowSlot>,
    pub scroll: Vec<RowSlot>,
    pub frozen_offset_y: i32,
}

impl RowSlot {
    #[inline]
    pub fn bottom(&self) -> i32 {
        self.top + self.height
    }
}

impl ColSlot {
    #[inline]
    pub fn right(&self) -> i32 {
        self.left + self.width
    }
}
