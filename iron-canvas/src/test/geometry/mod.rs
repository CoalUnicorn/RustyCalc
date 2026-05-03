#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Line, Point, Span};
use crate::geometry::utils::col_name;
use crate::CanvasSize;

#[test]
fn backing_size_scales_by_dpr() {
    assert_eq!(
        CanvasSize { w: 100.0, h: 200.0 }.to_backing_size(2.0),
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
        .to_backing_size(1.0),
        (1920, 1080)
    );
}

#[test]
fn backing_size_truncates_fractional_pixels() {
    // (100.3 * 1.5) = 150.45 → truncates to 150; (50.7 * 1.5) = 76.05 → 76
    assert_eq!(
        CanvasSize { w: 100.3, h: 50.7 }.to_backing_size(1.5),
        (150, 76)
    );
}

#[test]
fn right_is_x_plus_width() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 15.0,
    };
    assert_eq!(rect.right(), 25.0);
}

#[test]
fn bottom_is_y_plus_height() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        width: 20.0,
        height: 25.0,
    };
    assert_eq!(rect.bottom(), 35.0);
}

#[test]
fn top_left_returns_point_at_rect_origin() {
    let rect = PixelRect {
        top_left: Point { x: 5.0, y: 10.0 },
        height: 20.0,
        width: 15.0,
    };
    assert_eq!(rect.top_left(), Point { x: 5.0, y: 10.0 });
}

#[test]
fn center_returns_midpoint() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 10.0 },
        width: 30.0,
        height: 30.0,
    };
    assert_eq!(rect.center(), Point { x: 25.0, y: 25.0 });
}

#[test]
fn inset_with_positive_values_shrinks_symmetrically() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 20.0 },
        width: 100.0,
        height: 50.0,
    };
    let inner = rect.inset(2.0, 3.0);
    assert_eq!(inner.top_left.x, 12.0);
    assert_eq!(inner.top_left.y, 23.0);
    assert_eq!(inner.width, 96.0);
    assert_eq!(inner.height, 44.0);
}

#[test]
fn inset_with_zero_is_identity() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 20.0 },
        width: 100.0,
        height: 50.0,
    };
    let inner = rect.inset(0.0, 0.0);
    assert_eq!(inner.top_left.x, 10.0);
    assert_eq!(inner.top_left.y, 20.0);
    assert_eq!(inner.width, 100.0);
    assert_eq!(inner.height, 50.0);
}

#[test]
fn inset_with_negative_values_grows_rect() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 20.0 },
        width: 100.0,
        height: 50.0,
    };
    let inner = rect.inset(-10.0, -10.0);
    assert_eq!(inner.top_left.x, 0.0);
    assert_eq!(inner.top_left.y, 10.0);
    assert_eq!(inner.width, 120.0);
    assert_eq!(inner.height, 70.0);
}

#[test]
fn inset_preserves_center() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 20.0 },
        width: 100.0,
        height: 50.0,
    };
    let inner = rect.inset(50.0, 100.0);
    assert_eq!(rect.center(), inner.center());
}

#[test]
fn intersects_true_when_rect_inside_canvas() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 10.0 },
        width: 50.0,
        height: 50.0,
    };
    assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_true_when_rect_straddles_edge() {
    let rect = PixelRect {
        top_left: Point { x: -10.0, y: -10.0 },
        width: 50.0,
        height: 50.0,
    };
    assert!(rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_off_right() {
    let rect = PixelRect {
        top_left: Point { x: 250.0, y: 10.0 },
        width: 50.0,
        height: 50.0,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_off_left() {
    let rect = PixelRect {
        top_left: Point { x: -100.0, y: 10.0 },
        width: 50.0,
        height: 50.0,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn intersects_false_when_rect_below_canvas() {
    let rect = PixelRect {
        top_left: Point { x: 10.0, y: 250.0 },
        width: 50.0,
        height: 50.0,
    };
    assert!(!rect.intersects(CanvasSize { w: 200.0, h: 200.0 }));
}

#[test]
fn horizontal_line_offsets_y_by_delta() {
    //
    let line = Line::H {
        span: Span {
            from: 0.0,
            to: 10.0,
        },
        y: 5.0,
    };
    assert_eq!(
        line.offset_cross(2.0),
        Line::H {
            span: Span {
                from: 0.0,
                to: 10.0,
            },
            y: 7.0,
        }
    );
}

#[test]
fn vertical_line_offsets_x_by_delta() {
    let line = Line::V {
        span: Span {
            from: 0.0,
            to: 10.0,
        },
        x: 5.0,
    };
    assert_eq!(
        line.offset_cross(2.0),
        Line::V {
            span: Span {
                from: 0.0,
                to: 10.0,
            },
            x: 7.0,
        }
    );
}

#[test]
fn offset_cross_with_zero_is_identity() {
    let span = Span {
        from: 0.0,
        to: 10.0,
    };
    let h = Line::H { span, y: 5.0 };
    assert_eq!(h.offset_cross(0.0), h);
    let v = Line::V { x: 5.0, span };
    assert_eq!(v.offset_cross(0.0), v);
}

#[test]
fn offset_cross_with_negative_shifts_opposite() {
    let span = Span {
        from: 0.0,
        to: 10.0,
    };
    assert_eq!(
        Line::H { span, y: 5.0 }.offset_cross(-2.0),
        Line::H { span, y: 3.0 }
    );
    assert_eq!(
        Line::V { x: 5.0, span }.offset_cross(-2.0),
        Line::V { x: 3.0, span }
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
