use std::fmt::{self, Display};

use serde::{Deserialize, Serialize};

use crate::{CanvasSize, geometry::prim::Point};

/// A rectangle in logical (CSS) pixels on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PixelRect {
    pub top_left: Point,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    pub fn left(&self) -> i32 {
        self.top_left.x
    }
    pub fn top(&self) -> i32 {
        self.top_left.y
    }
    pub fn right(&self) -> i32 {
        self.top_left.x + self.width
    }
    pub fn bottom(&self) -> i32 {
        self.top_left.y + self.height
    }

    /// Return the positive-area overlap between two pixel rectangles.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (left < right && top < bottom).then_some(Self {
            top_left: Point { x: left, y: top },
            width: right - left,
            height: bottom - top,
        })
    }

    /// Smallest rectangle containing both inputs.
    pub fn bounding_union(self, other: Self) -> Self {
        let left = self.left().min(other.left());
        let top = self.top().min(other.top());
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            top_left: Point { x: left, y: top },
            width: right - left,
            height: bottom - top,
        }
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.left() + self.width / 2,
            y: self.top() + self.height / 2,
        }
    }
    /// Shrink by `dx` / `dy` on each side (negative values grow the rect).
    ///
    /// # Examples
    ///
    /// ```
    /// use iron_canvas_core::{PixelRect, Point};
    /// let r = PixelRect { top_left: Point { x: 10, y: 20 }, width: 100, height: 50 };
    ///
    /// let shrunk = r.inset(2, 3);
    /// assert_eq!(shrunk.top_left, Point { x: 12, y: 23 });
    /// assert_eq!((shrunk.width, shrunk.height), (96, 44));
    ///
    /// let grown = r.inset(-1, -1);
    /// assert_eq!(grown.top_left, Point { x: 9, y: 19 });
    /// assert_eq!((grown.width, grown.height), (102, 52));
    /// ```
    pub fn inset(&self, dx: i32, dy: i32) -> Self {
        Self {
            top_left: Point {
                x: self.left() + dx,
                y: self.top() + dy,
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
        f64::from(self.left()) < canvas.w
            && self.right() > 0
            && f64::from(self.top()) < canvas.h
            && self.bottom() > 0
    }

    /// The four corners in clockwise order from `top_left`: TL, TR, BR, BL.
    /// Used by `PainterShapes::fill_rect` to express a rect as a closed polygon.
    pub fn corners(&self) -> [Point; 4] {
        let (x, y) = (self.left(), self.top());
        [
            Point { x, y },
            Point {
                x: x + self.width,
                y,
            },
            Point {
                x: x + self.width,
                y: y + self.height,
            },
            Point {
                x,
                y: y + self.height,
            },
        ]
    }

    pub fn as_f64_tuple(self) -> (f64, f64, f64, f64) {
        (
            f64::from(self.left()),
            f64::from(self.top()),
            f64::from(self.width),
            f64::from(self.height),
        )
    }
}

/// Inline-CSS layout string (`left:Xpx;top:Ypx;width:Wpx;height:Hpx;`),
/// useful for setting a Leptos element's `style` attribute.
///
/// # Examples
///
/// ```
/// use iron_canvas_core::{PixelRect, Point};
/// let r = PixelRect { top_left: Point { x: 10, y: 20 }, width: 100, height: 50 };
/// assert_eq!(r.to_string(), "left:10px;top:20px;width:100px;height:50px;");
/// ```
impl Display for PixelRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px;",
            self.left(),
            self.top(),
            self.width,
            self.height
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> PixelRect {
        PixelRect {
            top_left: Point { x, y },
            width,
            height,
        }
    }

    #[test]
    fn intersection_returns_positive_overlap() {
        assert_eq!(
            rect(1, 2, 8, 7).intersection(rect(5, 4, 8, 9)),
            Some(rect(5, 4, 4, 5))
        );
    }

    #[test]
    fn intersection_rejects_edge_only_contact_and_disjoint_rects() {
        assert_eq!(rect(0, 0, 5, 5).intersection(rect(5, 0, 4, 4)), None);
        assert_eq!(rect(0, 0, 5, 5).intersection(rect(8, 8, 2, 2)), None);
    }

    #[test]
    fn intersection_clamps_to_canvas_bounds() {
        assert_eq!(
            rect(-3, -4, 10, 12).intersection(rect(0, 0, 100, 50)),
            Some(rect(0, 0, 7, 8))
        );
    }

    #[test]
    fn bounding_union_contains_both_rects() {
        assert_eq!(
            rect(4, 8, 5, 3).bounding_union(rect(-2, 10, 4, 9)),
            rect(-2, 8, 11, 11)
        );
    }
}
