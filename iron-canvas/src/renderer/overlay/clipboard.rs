//! Clipboard marching-ants border around the last Ctrl+C copied range.
//! No-op when the clipboard is empty or lives on another sheet.

use super::DashFill;
use crate::chrome::Chrome;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::types::coord::SheetArea;
use crate::CanvasModel;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_clipboard_overlay(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        clipboard: Option<&SheetArea>,
    ) {
        let sheet = model.get_selected_sheet();
        let Some(cb) = clipboard else { return };
        if cb.sheet != sheet {
            return;
        }
        self.draw_dashed_range(
            frame,
            cb.range.normalized(),
            PaintColor::from_theme_str(&frame.theme.selection_color),
            DashFill::Outline,
        );
    }
}
