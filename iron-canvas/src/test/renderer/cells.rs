#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{BorderEdge, Line, Point, Span};

#[test]
fn left_edge_is_vertical_line_at_rect_x() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(
        BorderEdge::Left.line(rect),
        Line::V {
            x: 5,
            span: Span { from: 10, to: 25 }
        }
    );
}

#[test]
fn right_edge_is_vertical_line_at_rect_right() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(
        BorderEdge::Right.line(rect),
        Line::V {
            x: 25,
            span: Span { from: 10, to: 25 },
        }
    )
}

#[test]
fn top_edge_is_horizontal_line_at_rect_y() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(
        BorderEdge::Top.line(rect),
        Line::H {
            y: 10,
            span: Span { from: 5, to: 25 },
        }
    );
}

#[test]
fn bottom_edge_is_horizontal_line_at_rect_bottom() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(
        BorderEdge::Bottom.line(rect),
        Line::H {
            y: 25,
            span: Span { from: 5, to: 25 },
        }
    );
}
