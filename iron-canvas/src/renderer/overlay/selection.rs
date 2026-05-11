//! Blue selection border + semi-transparent fill + autofill handle for
//! the active selection.

use crate::chrome::Chrome;
use crate::geometry::constants::{AUTOFILL_HANDLE_BORDER_PX, SELECTION_BORDER_WIDTH};
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::CanvasModel;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_selection(&self, model: &dyn CanvasModel, frame: &Chrome) {
        let Some(view) = model.get_selected_view() else {
            return;
        };
        let Some(cell) = frame.range_rect(view.selection.normalized()) else {
            return;
        };

        self.painter.rect_fill(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_fill),
        );

        // Restore the active cell's fill + borders + text on top of the
        // selection tint so its actual style shows through while selected.
        // Sheet is implicit — taken from `frame.sheet`.
        self.repaint_active_cell(model, view.row, view.column, frame);

        self.painter.rect_stroke(
            cell,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(SELECTION_BORDER_WIDTH),
        );

        // Autofill handle: top-left at the selection's bottom-right
        // corner (Excel anchor — pokes outside the selection). Filled
        // with selection_color, ringed in cell_bg so it stays visible
        // against any cell fill underneath. Skips full-row/col selections
        // — handle_rect returns None there, matching `autofill_handle()`
        // semantics.
        if let Some(handle) = frame.autofill_handle_rect() {
            self.painter.rect_fill(
                handle,
                PaintColor::from_theme_str(&frame.theme.selection_color),
            );
            self.painter.rect_stroke(
                handle,
                PaintColor::from_theme_str(&frame.theme.cell_bg),
                f64::from(AUTOFILL_HANDLE_BORDER_PX),
            );
        }
    }
}
