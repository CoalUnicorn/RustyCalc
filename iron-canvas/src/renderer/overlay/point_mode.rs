//! Point-mode range highlight — blue dashed outline with an 8% fill tint.

use crate::chrome::Chrome;
use super::DashFill;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::types::coord::RCRange;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_point_overlay(&self, frame: &Chrome, point_range: Option<RCRange>) {
        let Some(pr) = point_range else { return };
        self.draw_dashed_range(
            frame,
            pr.normalized(),
            PaintColor::from_theme_str(&frame.theme.pointing),
            DashFill::Tinted(frame.theme.pointing_tint.clone()),
        );
    }
}
