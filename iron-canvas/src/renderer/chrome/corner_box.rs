//! Top-left blank square plus the two axis lines that separate the
//! header strips from the cell area.

use crate::chrome::Chrome;
use crate::geometry::constants::STANDARD_BORDER_WIDTH;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Point, Span};
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;

impl<P: Painter> RendererCore<P> {
    pub(crate) fn draw_corner_box(&self, frame: &Chrome) {
        let corner = PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: frame.row_header_thickness,
            height: frame.col_header_thickness,
        };
        self.painter
            .rect_fill(corner, PaintColor::from_theme_str(&frame.theme.header_bg));

        // Stroke the 1-px chrome border on a `.5` centerline so the whole
        // pixel under the header strip's outer edge is covered (sharp line,
        // no anti-alias bleed into the cell area). `cell_origin` sits
        // `CELL_AREA_INSET` past the header thickness, so the gap between
        // chrome border and the first cell pixel is what keeps the selection
        // stroke at row 1 / col A from sinking into chrome.
        let border_color = PaintColor::from_theme_str(&frame.theme.header_border_color);
        self.painter.stroke_hline(
            Span {
                from: 0,
                to: frame.canvas_size.w as i32,
            },
            f64::from(frame.col_header_thickness) + 0.5,
            border_color,
            f64::from(STANDARD_BORDER_WIDTH),
        );
        self.painter.stroke_vline(
            f64::from(frame.row_header_thickness) + 0.5,
            Span {
                from: 0,
                to: frame.canvas_size.h as i32,
            },
            border_color,
            f64::from(STANDARD_BORDER_WIDTH),
        );
    }
}
