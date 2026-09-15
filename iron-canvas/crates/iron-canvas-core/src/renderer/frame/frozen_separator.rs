//! Thick separator strokes between frozen bands and the scrollable grid.
//!
//! Painted *after* cells so the divider wins its pixels over the
//! rightmost/bottommost frozen cell's grid stroke.

use crate::chrome::Chrome;
use crate::geometry::constants::FROZEN_SEP;
use crate::geometry::prim::Span;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;

impl<P: Painter> RendererCore<P> {
    pub fn draw_frozen_separators(&self, frame: &Chrome) {
        let p = &frame.pane_set;
        let frozen_rows = p.rows.frozen_count();
        let frozen_cols = p.cols.frozen_count();
        if frozen_rows == 0 && frozen_cols == 0 {
            return;
        }
        let color = PaintColor::from_theme_str(&frame.theme.grid_separator_color);
        let width = f64::from(FROZEN_SEP);
        // Centerline = midpoint of the FROZEN_SEP gap between the last frozen
        // slot and the first scroll slot, so the stroke fills the gap exactly
        // and aligns with the unfilled gap in the header strips. For odd
        // FROZEN_SEP this also lands on a `.5` boundary — the crisp pixel
        // position for odd-width Canvas2D strokes.
        let half_sep = f64::from(FROZEN_SEP) / 2.0;
        let sep_y = f64::from(p.rows.frozen_offset) - half_sep;
        let sep_x = f64::from(p.cols.frozen_offset) - half_sep;
        let (canvas_w, canvas_h) = frame.canvas_size.to_logical_extent();

        if frozen_rows > 0 {
            self.painter.stroke_hline(
                Span {
                    from: 0,
                    to: canvas_w,
                },
                sep_y,
                color,
                width,
            );
        }
        if frozen_cols > 0 {
            self.painter.stroke_vline(
                sep_x,
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
