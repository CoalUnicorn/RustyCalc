//! Hover-highlight decoration — the proof consumer of the open decoration
//! registry (`Orchestrator::add_decoration`).
//!
//! The facade keeps a typed `Rc<HoverLayer>` handle and mutates through
//! `set_cell`; the registry holds the same layer as `Rc<dyn Layer>` and
//! paints it above every built-in. Compare-then-raise is the consumer's
//! job for custom layers, so `set_cell` reports whether anything changed
//! and the caller raises `request_overlay_repaint` only then.

use std::cell::Cell;

use iron_canvas_core::Layer;
use iron_canvas_core::chrome::Chrome;
use iron_canvas_core::painter::{GroupClass, PaintColor, Painter};

#[derive(Default)]
pub struct HoverLayer {
    /// 1-based engine coords of the hovered cell; `None` paints nothing.
    cell: Cell<Option<(i32, i32)>>,
}

impl HoverLayer {
    /// JS-facing 0-based hover coords -> engine cell. Any negative
    /// coordinate means "pointer left the grid" and clears the hover.
    pub fn cell_from_js(row: i32, col: i32) -> Option<(i32, i32)> {
        (row >= 0 && col >= 0).then(|| (row + 1, col + 1))
    }

    /// Compare-and-set. Returns whether the state changed — the caller
    /// raises the overlay repaint only then.
    pub fn set_cell(&self, cell: Option<(i32, i32)>) -> bool {
        let changed = self.cell.get() != cell;
        if changed {
            self.cell.set(cell);
        }
        changed
    }
}

impl Layer for HoverLayer {
    fn group(&self) -> GroupClass {
        GroupClass::Custom
    }

    // Paint-only: no `hit_test` override, so the hover ring never shadows
    // the built-in hit zones beneath it.
    fn paint(&self, frame: &Chrome, painter: &dyn Painter) {
        let Some((row, col)) = self.cell.get() else {
            return;
        };
        let Some(rect) = frame.cell_rect(row, col) else {
            return;
        };
        painter.rect_stroke(
            rect,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            1.0,
        );
    }
}
