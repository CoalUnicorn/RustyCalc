//! Overlay-canvas decorations: selection, autofill preview, clipboard
//! marching ants, point-mode range, formula-ref highlights.
//!
//! Each decoration owns its own state and implements `Layer`. `OverlayLayer`
//! iterates them in fixed z-order on paint and reverse-z on hit-test.

use crate::chrome::Chrome;
use crate::painter::Painter;
use crate::types::coord::RCRange;
use crate::types::ui::HitTest;
use crate::CanvasModel;

pub(crate) mod autofill;
pub(crate) mod clipboard;
pub(crate) mod formula_refs;
pub(crate) mod point_mode;
pub(crate) mod selection;

// Per-layer struct re-exports are added by tasks A2-A4 as each layer is
// ported. Today the only public surface from this module is the `Layer`
// trait and the `RepaintActiveCell` hook payload.

pub(crate) trait Layer {
    fn paint(&self, model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter);

    fn hit_test(
        &self,
        _frame: &Chrome,
        _selection_range: RCRange,
        _x: i32,
        _y: i32,
    ) -> Option<HitTest> {
        None
    }

    /// Selection's fill paints under the active-cell repaint; its stroke
    /// + handle paint over it. `paint` runs first, the renderer hook fires,
    /// then `paint_after_hook` finishes the layer. Default no-op covers
    /// every decoration except `SelectionLayer`.
    fn paint_after_hook(
        &self,
        _model: &dyn CanvasModel,
        _frame: &Chrome,
        _painter: &dyn Painter,
    ) {
    }

    fn after_paint_renderer_hook(
        &self,
        _model: &dyn CanvasModel,
        _frame: &Chrome,
    ) -> Option<RepaintActiveCell> {
        None
    }
}

pub(crate) struct RepaintActiveCell {
    pub(crate) row: i32,
    pub(crate) col: i32,
}
