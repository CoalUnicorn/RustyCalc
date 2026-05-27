//! Clipboard marching-ants border around the last Ctrl+C copied range.
//! No-op when the clipboard is empty or lives on another sheet.

use crate::chrome::Chrome;
use crate::decoration::Layer;
use crate::geometry::constants::DASHED_BORDER_WIDTH;
use crate::painter::{GroupClass, PaintColor, Painter};
use crate::types::coord::SheetArea;

#[derive(Default)]
pub struct ClipboardLayer {
    pub clipboard: Option<SheetArea>,
}

impl Layer for ClipboardLayer {
    fn group(&self) -> GroupClass {
        GroupClass::Clipboard
    }

    fn paint(&self, frame: &Chrome, painter: &dyn Painter) {
        let Some(cb) = self.clipboard.as_ref() else {
            return;
        };
        if cb.sheet != frame.sheet {
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
