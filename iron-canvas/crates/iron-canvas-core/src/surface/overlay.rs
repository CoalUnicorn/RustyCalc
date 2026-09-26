use std::rc::Rc;

use crate::CanvasModel;
use crate::chrome::Chrome;
use crate::decoration::selection::SelectionLayer;
use crate::decoration::{DecorationId, Layer};
use crate::geometry::prim::Axis;
use crate::painter::{GroupClass, Painter};
use crate::renderer::OverlayRenderer;

use super::{LayerBase, Surface, full_canvas_rect};

// Overlay-layer specialization. The clear is a `Painter::clear_rect` so it
// routes through every backend uniformly; SVG / Recorder no-op / record it.
impl<S> LayerBase<S, OverlayRenderer<S::P>>
where
    S: Surface,
{
    pub fn paint_overlay_layer(
        &mut self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        selection: &SelectionLayer,
        others: &[&dyn Layer],
        customs: &[(DecorationId, Rc<dyn Layer>)],
    ) {
        let size = frame.canvas_size();
        let painter = self.surface.painter();
        painter.clear_rect(full_canvas_rect(size));
        painter.begin_group(GroupClass::Overlay);

        // Selection paints fill (under) then stroke + handle (over) the
        // active-cell repaint. Header highlights land between selection
        // and the rest so the highlighted header strip is above the
        // selection tint.
        painter.begin_group(GroupClass::SelectionFill);
        selection.paint(frame, painter);
        painter.end_group();

        // Gate on `Some` so a tick where the model briefly has no selected
        // view (sheet swap, workbook reload) does not repaint A1 with the
        // default-zero snapshot or emit an empty bracket into recordings.
        if let Some(cell) = selection.active_cell_repaint() {
            painter.begin_group(GroupClass::ActiveCellRepaint);
            self.renderer.repaint_active_cell(model, cell, frame);
            painter.end_group();
        }

        painter.begin_group(GroupClass::SelectionStroke);
        selection.paint_stroke(frame, painter);
        painter.end_group();

        if let Some(sel) = selection.selection_range {
            painter.begin_group(GroupClass::HeaderHighlights);
            if frame.row_header_thickness > 0 {
                self.renderer
                    .render_header_highlights(Axis::Row, frame, sel);
            }
            if frame.col_header_thickness > 0 {
                self.renderer
                    .render_header_highlights(Axis::Column, frame, sel);
            }
            painter.end_group();
        }

        // Other decorations: one group each, named by the layer itself.
        for layer in others {
            painter.begin_group(layer.group());
            layer.paint(frame, painter);
            painter.end_group();
        }
        // Consumer band — topmost, insertion order back-to-front. Same
        // bracket-per-layer contract as the built-ins above.
        for (_, layer) in customs {
            painter.begin_group(layer.group());
            layer.paint(frame, painter);
            painter.end_group();
        }
        painter.end_group();
    }
}
