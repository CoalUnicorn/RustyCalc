//! Dashed preview of the autofill-handle drag target.

use crate::chrome::Chrome;
use crate::geometry::constants::STANDARD_BORDER_WIDTH;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::types::coord::{AutofillTarget, RCRange};
use crate::CanvasModel;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_extend_preview(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        target: AutofillTarget,
    ) {
        let Some(view) = model.get_selected_view() else {
            return;
        };
        let sel = view.selection.normalized();
        let range = RCRange {
            r1: sel.r1.min(target.row),
            c1: sel.c1.min(target.col),
            r2: sel.r2.max(target.row),
            c2: sel.c2.max(target.col),
        };
        let Some(b) = frame.range_rect(range) else {
            return;
        };

        self.painter.rect_dashed(
            b,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(STANDARD_BORDER_WIDTH),
        );
    }
}
