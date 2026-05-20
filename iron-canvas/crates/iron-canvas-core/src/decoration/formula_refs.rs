//! Per-reference formula highlights for the in-edit cell. One color per
//! `color_idx` (mod the palette), off-sheet refs silently skipped.

use std::borrow::Cow;

use crate::chrome::Chrome;
use crate::geometry::constants::DASHED_BORDER_WIDTH;
use crate::decoration::Layer;
use crate::painter::{PaintColor, Painter};
use crate::theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS};
use crate::types::coord::FormulaRef;
use crate::CanvasModel;

#[derive(Default)]
pub struct FormulaRefsLayer {
    pub refs: Vec<FormulaRef>,
}

impl Layer for FormulaRefsLayer {
    fn paint(&self, model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter) {
        if self.refs.is_empty() {
            return;
        }
        let sheet = model.get_selected_sheet();
        for fr in &self.refs {
            if fr.sheet_area.sheet != sheet {
                continue;
            }
            let Some(b) = frame.range_rect(fr.sheet_area.range.normalized()) else {
                continue;
            };
            let idx = fr.color_idx % FORMULA_REF_COLORS.len();
            painter.rect_fill(
                b,
                PaintColor::from_theme_str(&Cow::Borrowed(FORMULA_REF_TINTS[idx])),
            );
            painter.rect_dashed(
                b,
                PaintColor::Static(FORMULA_REF_COLORS[idx]),
                f64::from(DASHED_BORDER_WIDTH),
            );
        }
    }
}
