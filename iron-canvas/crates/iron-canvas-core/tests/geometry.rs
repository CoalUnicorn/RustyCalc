#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use iron_canvas_core::CanvasSize;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::geometry::utils::col_name;

#[test]
fn backing_size_scales_by_dpr() {
    assert_eq!(
        CanvasSize { w: 100.0, h: 200.0 }.to_backing_size(2),
        (200, 400)
    );
}

#[test]
fn backing_size_at_1x_dpr_equals_css() {
    assert_eq!(
        CanvasSize {
            w: 1920.0,
            h: 1080.0
        }
        .to_backing_size(1),
        (1920, 1080)
    );
}

#[test]
fn backing_size_truncates_fractional_pixels() {
    assert_eq!(
        CanvasSize { w: 100.3, h: 50.7 }.to_backing_size(2),
        (200, 101)
    );
}

#[test]
fn right_is_x_plus_width() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 15,
    };
    assert_eq!(rect.right(), 25);
}

#[test]
fn bottom_is_y_plus_height() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        width: 20,
        height: 25,
    };
    assert_eq!(rect.bottom(), 35);
}

#[test]
fn top_left_returns_point_at_rect_origin() {
    let rect = PixelRect {
        top_left: Point { x: 5, y: 10 },
        height: 20,
        width: 15,
    };
    assert_eq!(rect.top_left, Point { x: 5, y: 10 });
}

#[test]
fn center_returns_midpoint() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 10 },
        width: 30,
        height: 30,
    };
    assert_eq!(rect.center(), Point { x: 25, y: 25 });
}

#[test]
fn inset_with_positive_values_shrinks_symmetrically() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 20 },
        width: 100,
        height: 50,
    };
    let inner = rect.inset(2, 3);
    assert_eq!(inner.top_left.x, 12);
    assert_eq!(inner.top_left.y, 23);
    assert_eq!(inner.width, 96);
    assert_eq!(inner.height, 44);
}

#[test]
fn inset_with_zero_is_identity() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 20 },
        width: 100,
        height: 50,
    };
    let inner = rect.inset(0, 0);
    assert_eq!(inner.top_left.x, 10);
    assert_eq!(inner.top_left.y, 20);
    assert_eq!(inner.width, 100);
    assert_eq!(inner.height, 50);
}

#[test]
fn inset_with_negative_values_grows_rect() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 20 },
        width: 100,
        height: 50,
    };
    let inner = rect.inset(-10, -10);
    assert_eq!(inner.top_left.x, 0);
    assert_eq!(inner.top_left.y, 10);
    assert_eq!(inner.width, 120);
    assert_eq!(inner.height, 70);
}

#[test]
fn inset_preserves_center() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 20 },
        width: 100,
        height: 50,
    };
    let inner = rect.inset(50, 100);
    assert_eq!(rect.center(), inner.center());
}

#[test]
fn intersects_true_when_rect_inside_canvas() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 10 },
        width: 50,
        height: 50,
    };
    assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_true_when_rect_straddles_edge() {
    let rect = PixelRect {
        top_left: Point { x: -10, y: -10 },
        width: 50,
        height: 50,
    };
    assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_off_right() {
    let rect = PixelRect {
        top_left: Point { x: 250, y: 10 },
        width: 50,
        height: 50,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_off_left() {
    let rect = PixelRect {
        top_left: Point { x: -100, y: 10 },
        width: 50,
        height: 50,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_below_canvas() {
    let rect = PixelRect {
        top_left: Point { x: 10, y: 250 },
        width: 50,
        height: 50,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn horizontal_line_offsets_y_by_delta() {
    //
    let line = Line::H {
        span: Span { from: 0, to: 10 },
        y: 5,
    };
    assert_eq!(
        line.offset_cross(2),
        Line::H {
            span: Span { from: 0, to: 10 },
            y: 7,
        }
    );
}

#[test]
fn vertical_line_offsets_x_by_delta() {
    let line = Line::V {
        span: Span { from: 0, to: 10 },
        x: 5,
    };
    assert_eq!(
        line.offset_cross(2),
        Line::V {
            span: Span { from: 0, to: 10 },
            x: 7,
        }
    );
}

#[test]
fn offset_cross_with_zero_is_identity() {
    let span = Span { from: 0, to: 10 };
    let h = Line::H { span, y: 5 };
    assert_eq!(h.offset_cross(0), h);
    let v = Line::V { x: 5, span };
    assert_eq!(v.offset_cross(0), v);
}

#[test]
fn offset_cross_with_negative_shifts_opposite() {
    let span = Span { from: 0, to: 10 };
    assert_eq!(
        Line::H { span, y: 5 }.offset_cross(-2),
        Line::H { span, y: 3 }
    );
    assert_eq!(
        Line::V { x: 5, span }.offset_cross(-2),
        Line::V { x: 3, span }
    );
}

#[test]
fn col_name_one_is_a() {
    assert_eq!(col_name(1), "A");
}

#[test]
fn col_name_26_is_z() {
    assert_eq!(col_name(26), "Z");
}

#[test]
fn col_name_707_is_zz() {
    assert_eq!(col_name(707), "AAE");
}

#[test]
fn col_name_zero_returns_empty_string() {
    assert_eq!(col_name(0), "");
}
