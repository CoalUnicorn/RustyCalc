//! Clipboard marching-ants border around the last Ctrl+C copied range.
//! No-op when the clipboard is empty or lives on another sheet.

use crate::chrome::Chrome;
use crate::decoration::Layer;
use crate::geometry::constants::DASHED_BORDER_WIDTH;
use crate::painter::{PaintColor, Painter};
use crate::types::coord::SheetArea;
use crate::CanvasModel;

#[derive(Default)]
pub struct ClipboardLayer {
    pub clipboard: Option<SheetArea>,
}

impl Layer for ClipboardLayer {
    fn paint(&self, model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter) {
        let Some(cb) = self.clipboard.as_ref() else {
            return;
        };
        if cb.sheet != model.get_selected_sheet() {
            return;
        }
        let Some(b) = frame.range_rect(cb.range.normalized()) else {
            return;
        };
        painter.rect_dashed(
            b,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(DASHED_BORDER_WIDTH),
        );
    }
}
