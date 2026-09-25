#[derive(Clone, Copy, Debug)]
pub struct RowSlot {
    pub row: i32,
    /// Absolute canvas Y, not relative to any pane.
    pub top: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct ColSlot {
    pub col: i32,
    /// Absolute canvas X, not relative to any pane.
    pub left: i32,
    pub width: i32,
}

/// Axis-symmetric slot view. Lets every walk over `PaneSet`'s slot vecs share
/// one implementation regardless of axis. `end` defaults to `start + extent`,
/// giving generic callers the far edge (row bottom / col right) without an
/// axis-specific accessor.
pub trait AxisSlot: Sized {
    fn new(id: i32, start: i32, extent: i32) -> Self;
    fn id(&self) -> i32;
    fn start(&self) -> i32;
    fn extent(&self) -> i32;
    #[inline]
    fn end(&self) -> i32 {
        self.start() + self.extent()
    }
}

impl AxisSlot for RowSlot {
    #[inline]
    fn new(id: i32, start: i32, extent: i32) -> Self {
        RowSlot {
            row: id,
            top: start,
            height: extent,
        }
    }
    #[inline]
    fn id(&self) -> i32 {
        self.row
    }
    #[inline]
    fn start(&self) -> i32 {
        self.top
    }
    #[inline]
    fn extent(&self) -> i32 {
        self.height
    }
}

impl AxisSlot for ColSlot {
    #[inline]
    fn new(id: i32, start: i32, extent: i32) -> Self {
        ColSlot {
            col: id,
            left: start,
            width: extent,
        }
    }
    #[inline]
    fn id(&self) -> i32 {
        self.col
    }
    #[inline]
    fn start(&self) -> i32 {
        self.left
    }
    #[inline]
    fn extent(&self) -> i32 {
        self.width
    }
}
