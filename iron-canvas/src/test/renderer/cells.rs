use crate::geometry::BorderEdge;
use crate::Line;
use crate::PixelRect;
use crate::Point;
use crate::Span;

#[test]
fn left_edge_is_vertical_line_at_rect_x() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 15.0,
    };
    assert_eq!(
        BorderEdge::Left.line(rect),
        Line::V {
            x: 5.0,
            span: Span {
                from: 10.0,
                to: 25.0,
            }
        }
    );
}

#[test]
fn right_edge_is_vertical_line_at_rect_right() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 15.0,
    };
    assert_eq!(
        BorderEdge::Right.line(rect),
        Line::V {
            x: 25.0,
            span: Span {
                from: 10.0,
                to: 25.0,
            },
        }
    )
}

#[test]
fn top_edge_is_horizontal_line_at_rect_y() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 15.0,
    };
    assert_eq!(
        BorderEdge::Top.line(rect),
        Line::H {
            y: 10.0,
            span: Span {
                from: 5.0,
                to: 25.0,
            },
        }
    );
}

#[test]
fn bottom_edge_is_horizontal_line_at_rect_bottom() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 15.0,
    };
    assert_eq!(
        BorderEdge::Bottom.line(rect),
        Line::H {
            y: 25.0,
            span: Span {
                from: 5.0,
                to: 25.0,
            },
        }
    );
}
