#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use iron_canvas_core::Side;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};

#[test]
fn all_sides_preserve_explicit_stroke_order() {
    assert_eq!(
        Side::ALL,
        [Side::Left, Side::Top, Side::Right, Side::Bottom]
    );
}

#[test]
fn left_edge_is_vertical_line_at_rect_x() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(
        Side::Left.line(rect),
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
        Side::Right.line(rect),
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
        Side::Top.line(rect),
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
        Side::Bottom.line(rect),
        Line::H {
            y: 25,
            span: Span { from: 5, to: 25 },
        }
    );
}
