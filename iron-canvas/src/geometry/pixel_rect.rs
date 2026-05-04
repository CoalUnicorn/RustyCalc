use std::fmt::{self, Display};

use crate::{geometry::prim::Point, CanvasSize};

/// A rectangle in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PixelRect {
    pub top_left: Point,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    pub fn right(&self) -> i32 {
        self.top_left.x + self.width
    }
    pub fn bottom(&self) -> i32 {
        self.top_left.y + self.height
    }

    #[cfg(test)]
    pub fn top_left(&self) -> Point {
        self.top_left
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.top_left.x + self.width / 2,
            y: self.top_left.y + self.height / 2,
        }
    }
    /// Shrink by `dx` / `dy` on each side (negative values grow the rect).
    pub fn inset(&self, dx: i32, dy: i32) -> Self {
        Self {
            top_left: Point {
                x: self.top_left.x + dx,
                y: self.top_left.y + dy,
            },

            width: self.width - 2 * dx,
            height: self.height - 2 * dy,
        }
    }

    /// True when this rect overlaps the canvas drawable area at all.
    /// Pure pixel-space AABB test - used inside per-cell loops to skip cells
    /// that fall off-canvas (notably when a frozen band is wider/taller than
    /// the canvas itself).
    pub fn intersects(&self, canvas: CanvasSize) -> bool {
        f64::from(self.top_left.x) < canvas.w
            && self.right() > 0
            && f64::from(self.top_left.y) < canvas.h
            && self.bottom() > 0
    }

    pub fn as_f64_tuple(self) -> (f64, f64, f64, f64) {
        (
            f64::from(self.top_left.x),
            f64::from(self.top_left.y),
            f64::from(self.width),
            f64::from(self.height),
        )
    }
}

impl Display for PixelRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px;",
            self.top_left.x, self.top_left.y, self.width, self.height
        )
    }
}
