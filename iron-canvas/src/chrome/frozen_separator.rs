//! Thick separator strokes between frozen bands and the scrollable grid.
//!
//! Painted *after* cells so the divider wins its pixels over the
//! rightmost/bottommost frozen cell's grid stroke.

use crate::chrome::Chrome;
use crate::geometry::constants::{FROZEN_SEP, HEADER_OFFSET};
use crate::geometry::prim::Span;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_frozen_separators(&self, frame: &Chrome) {
        let frc = &frame.frozen;
        if frc.rows == 0 && frc.cols == 0 {
            return;
        }
        let color = PaintColor::from_theme_str(&frame.theme.grid_separator_color);
        let width = f64::from(FROZEN_SEP);
        let sep_y = frc.offset.y - FROZEN_SEP / 2 + HEADER_OFFSET;
        let sep_x = frc.offset.x - FROZEN_SEP / 2 + HEADER_OFFSET;
        let canvas_w = frame.canvas_size.w as i32;
        let canvas_h = frame.canvas_size.h as i32;

        if frc.rows > 0 {
            self.painter.stroke_hline(
                Span {
                    from: 0,
                    to: canvas_w,
                },
                f64::from(sep_y),
                color,
                width,
            );
        }
        if frc.cols > 0 {
            self.painter.stroke_vline(
                f64::from(sep_x),
                Span {
                    from: 0,
                    to: canvas_h,
                },
                color,
                width,
            );
        }
    }
}
