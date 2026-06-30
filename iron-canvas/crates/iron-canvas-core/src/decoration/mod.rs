//! Overlay-canvas decorations: selection, autofill preview, clipboard
//! marching ants, point-mode range, formula-ref highlights.
//!
//! Each decoration owns its own snapshot state and implements `Layer`.
//! `LayerBase::paint_overlay_layer` wraps every decoration's `paint` call
//! in a `begin_group(layer.group()) ... end_group()` pair, so decoration
//! bodies stay free of group bookkeeping.
//!
//! `SelectionLayer` is special — it has a three-phase paint shape
//! (fill -> renderer's active-cell repaint -> stroke) orchestrated by name
//! in `LayerBase::paint_overlay_layer`. The extra phases live as
//! inherent methods on `SelectionLayer`, not on this trait.

use crate::chrome::Chrome;
use crate::painter::{GroupClass, Painter};
use crate::types::coord::RCRange;
use crate::types::ui::HitTest;

pub mod autofill;
pub mod clipboard;
pub(crate) mod decorations;
pub mod formula_refs;
pub mod point_mode;
pub mod selection;

pub use autofill::AutofillLayer;
pub use clipboard::ClipboardLayer;
pub use formula_refs::FormulaRefsLayer;
pub use point_mode::PointModeLayer;
pub use selection::{RepaintActiveCell, SelectionLayer};

pub use decorations::DecorationId;
pub(crate) use decorations::Decorations;

pub trait Layer {
    /// Stable tag for the `begin_group`/`end_group` wrapper the orchestrator
    /// emits around this decoration's paint pass.
    fn group(&self) -> GroupClass;

    /// Pure function of (own snapshot state, frame, painter). No model —
    /// overlay paints are paint-coherent with the snapshot, not the live
    /// model.
    fn paint(&self, frame: &Chrome, painter: &dyn Painter);

    /// Reverse-z hit probe. Default returns `None` (invisible to hit-tests).
    fn hit_test(
        &self,
        _frame: &Chrome,
        _selection_range: RCRange,
        _x: i32,
        _y: i32,
    ) -> Option<HitTest> {
        None
    }
}
