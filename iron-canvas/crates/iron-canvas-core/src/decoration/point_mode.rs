//! Point-mode range highlight — blue dashed outline with an 8% fill tint.

use crate::chrome::Chrome;
use crate::decoration::Layer;
use crate::geometry::constants::DASHED_BORDER_WIDTH;
use crate::painter::{PaintColor, Painter};
use crate::types::coord::RCRange;
use crate::CanvasModel;

#[derive(Default)]
pub struct PointModeLayer {
    pub point_range: Option<RCRange>,
}

impl Layer for PointModeLayer {
    fn paint(&self, _model: &dyn CanvasModel, frame: &Chrome, painter: &dyn Painter) {
        let Some(pr) = self.point_range else {
            return;
        };
        let Some(b) = frame.range_rect(pr.normalized()) else {
            return;
        };
        // Tint first so the dashed outline lands cleanly on top — the
        // 8% alpha would otherwise wash over and dim the dashes.
        painter.rect_fill(b, PaintColor::from_theme_str(&frame.theme.pointing_tint));
        painter.rect_dashed(
            b,
            PaintColor::from_theme_str(&frame.theme.pointing),
            f64::from(DASHED_BORDER_WIDTH),
        );
    }
}
