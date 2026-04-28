mod grid;
mod overlay;

pub(crate) use grid::GridLayer;
pub(crate) use overlay::OverlayLayer;

pub(crate) struct PaintGate {
    dirty: bool,
    #[cfg(test)]
    pub(crate) paint_count: u32,
}

impl PaintGate {
    pub(crate) fn new() -> Self {
        Self {
            dirty: false,
            #[cfg(test)]
            paint_count: 0,
        }
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn should_paint(&mut self) -> bool {
        let was_dirty = std::mem::replace(&mut self.dirty, false);
        #[cfg(test)]
        if was_dirty {
            self.paint_count += 1;
        }
        was_dirty
    }
}

#[cfg(test)]
mod tests {
    use super::PaintGate;

    #[test]
    fn fresh_gate_does_not_paint() {
        let mut gate = PaintGate::new();
        assert!(!gate.should_paint());
    }

    #[test]
    fn mark_dirty_enables_paint() {
        let mut gate = PaintGate::new();
        gate.mark_dirty();
        assert!(gate.should_paint());
    }

    #[test]
    fn should_paint_clears_flag() {
        let mut gate = PaintGate::new();
        gate.mark_dirty();
        gate.should_paint();
        assert!(!gate.should_paint(), "flag must be cleared after first should_paint");
    }

    #[test]
    fn double_mark_dirty_still_paints_once() {
        let mut gate = PaintGate::new();
        gate.mark_dirty();
        gate.mark_dirty();
        assert!(gate.should_paint());
        assert!(!gate.should_paint());
    }
}
