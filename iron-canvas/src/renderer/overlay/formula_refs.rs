//! Per-reference formula highlights for the in-edit cell. One color per
//! `color_idx` (mod the palette), off-sheet refs silently skipped.

use std::borrow::Cow;

use super::DashFill;
use crate::chrome::Chrome;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::theme::{FORMULA_REF_COLORS, FORMULA_REF_TINTS};
use crate::types::coord::FormulaRef;
use crate::CanvasModel;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_formula_ref_overlays(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        formula_refs: &[FormulaRef],
    ) {
        let sheet = model.get_selected_sheet();
        for fr in formula_refs {
            if fr.sheet_area.sheet != sheet {
                continue;
            }
            let idx = fr.color_idx % FORMULA_REF_COLORS.len();
            self.draw_dashed_range(
                frame,
                fr.sheet_area.range.normalized(),
                PaintColor::Static(FORMULA_REF_COLORS[idx]),
                DashFill::Tinted(Cow::Borrowed(FORMULA_REF_TINTS[idx])),
            );
        }
    }
}
