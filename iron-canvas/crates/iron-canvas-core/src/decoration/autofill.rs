//! Dashed preview of the autofill-handle drag target + autofill handle
//! hit-test.

use crate::chrome::Chrome;
use crate::decoration::Layer;
use crate::geometry::constants::{AUTOFILL_HIT_PAD_PX, STANDARD_BORDER_WIDTH};
use crate::painter::{GroupClass, PaintColor, Painter};
use crate::types::coord::{AutofillTarget, RCRange};
use crate::types::ui::HitTest;

#[derive(Default)]
pub struct AutofillLayer {
    pub extend_to: Option<AutofillTarget>,
    /// Snapshot of the selection rectangle the autofill drag extends from.
    /// Mirrored from `SelectionLayer::selection_range` by the orchestrator
    /// at refresh time so the preview is paint-coherent with the painted
    /// selection rather than chasing the live model.
    pub selection_range: RCRange,
}

impl Layer for AutofillLayer {
    fn group(&self) -> GroupClass {
        GroupClass::Autofill
    }

    fn paint(&self, frame: &Chrome, painter: &dyn Painter) {
        let Some(target) = self.extend_to else {
            return;
        };
        let sel = self.selection_range.normalized();
        let range = RCRange {
            r1: sel.r1.min(target.row),
            c1: sel.c1.min(target.col),
            r2: sel.r2.max(target.row),
            c2: sel.c2.max(target.col),
        };
        let Some(b) = frame.range_rect(range) else {
            return;
        };
        painter.rect_dashed(
            b,
            PaintColor::from_theme_str(&frame.theme.selection_color),
            f64::from(STANDARD_BORDER_WIDTH),
        );
    }

    fn hit_test(
        &self,
        frame: &Chrome,
        selection_range: RCRange,
        x: i32,
        y: i32,
    ) -> Option<HitTest> {
        let h = frame.autofill_handle_rect(selection_range)?;
        let pad = AUTOFILL_HIT_PAD_PX;
        if x < h.top_left.x - pad
            || x > h.right() + pad
            || y < h.top_left.y - pad
            || y > h.bottom() + pad
        {
            return None;
        }
        let row = frame.pane_set.pixel_to_row(y)?;
        let column = frame.pane_set.pixel_to_col(x)?;
        Some(HitTest::AutofillHandle { row, column })
    }
}
