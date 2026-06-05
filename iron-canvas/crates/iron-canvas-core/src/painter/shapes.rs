//! Convenience shapes layered over the `Painter::fill_path` primitive.
//! Follows the `Iterator`/`IteratorExt` + blanket-impl pattern: one
//! required primitive on `Painter`, N default-bodied conveniences here.

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::painter::{PaintColor, Painter};

pub trait PainterShapes: Painter {
    fn fill_triangle(&self, p1: Point, p2: Point, p3: Point, c: PaintColor) {
        self.fill_path(&[p1, p2, p3], c);
    }
    fn fill_rect(&self, r: PixelRect, c: PaintColor) {
        self.fill_path(&r.corners(), c);
    }
    fn fill_polygon(&self, pts: &[Point], c: PaintColor) {
        self.fill_path(pts, c);
    }
}

impl<T: Painter + ?Sized> PainterShapes for T {}
