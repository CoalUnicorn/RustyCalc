//! Test-only constructors shared by the crate's unit-test modules.

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;

/// `PixelRect` from its left/top/width/height, so a test reads
/// `pix(x, y, w, h)` instead of restating the `top_left: Point { .. }` field
/// every time.
pub(crate) fn pix(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
    PixelRect {
        top_left: Point { x, y },
        width: w,
        height: h,
    }
}
