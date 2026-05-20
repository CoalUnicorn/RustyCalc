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

pub mod autofill;
pub mod clipboard;
pub mod formula_refs;
pub mod point_mode;
pub mod selection;

pub use autofill::AutofillLayer;
pub use clipboard::ClipboardLayer;
pub use formula_refs::FormulaRefsLayer;
pub use point_mode::PointModeLayer;
pub use selection::SelectionLayer;

/// Overlay-canvas decoration. Each impl owns its own state (pushed in
/// from the orchestrator) and contributes one pass to `OverlayLayer::paint`,
/// plus an optional reverse-z hit-test probe.
///
/// `paint` and `paint_after_hook` bracket the optional active-cell repaint
/// `after_paint_renderer_hook` requests. The three-step shape exists for
/// `SelectionLayer`, which fills under the active cell and strokes over it;
/// every other decoration takes the default no-ops for the second and third
/// methods.
pub trait Layer {
    /// First (and usually only) paint pass. Runs in fixed z-order against
    /// the same painter every other decoration sees.
    fn paint(&self, model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter);

    /// Resolve a canvas-pixel hit to a `HitTest` variant this decoration
    /// owns. Probed in reverse z-order; the first `Some` wins. Default
    /// returns `None` (decoration is invisible to hit-tests).
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
    fn paint_after_hook(&self, _model: &dyn CanvasModel, _frame: &Chrome, _painter: &dyn Painter) {}

    /// Ask `OverlayLayer::paint` to run `OverlayRenderer::repaint_active_cell`
    /// between this decoration's `paint` and `paint_after_hook`. Returning
    /// `Some` is what gives `SelectionLayer` its under-fill / over-stroke
    /// sandwich; every other decoration returns `None`.
    fn after_paint_renderer_hook(
        &self,
        _model: &dyn CanvasModel,
        _frame: &Chrome,
    ) -> Option<RepaintActiveCell> {
        None
    }
}

/// Payload of `Layer::after_paint_renderer_hook` — the cell the renderer
/// must repaint before the requesting decoration finishes painting.
pub struct RepaintActiveCell {
    pub row: i32,
    pub col: i32,
}
