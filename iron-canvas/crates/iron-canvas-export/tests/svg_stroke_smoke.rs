//! `SvgPainter::stroke_path` writes every stroke attribute on the element
//! itself, so a styled path cannot leak a dash or cap into a later stroke.

#![cfg(feature = "svg")]

use iron_canvas_core::geometry::path::{Path, PathCmd, PointF};
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::Point;
use iron_canvas_core::painter::{LineCap, LineJoin, PaintColor, Painter, StrokeStyle};
use iron_canvas_export::SvgPainter;

fn rect(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
    PixelRect {
        top_left: Point { x, y },
        width: w,
        height: h,
    }
}

fn seg() -> [PathCmd; 2] {
    [
        PathCmd::Move(PointF::new(1.0, 2.0)),
        PathCmd::Line(PointF::new(9.0, 8.0)),
    ]
}

#[test]
fn stroke_path_emits_every_stroke_attribute() {
    let p = SvgPainter::new(100, 50);
    p.stroke_path(
        &Path::new(&seg()),
        PaintColor::Static("#ff0000"),
        &StrokeStyle {
            width: 2.5,
            cap: LineCap::Round,
            join: LineJoin::Bevel,
            miter_limit: 4.0,
            dash: &[3.0, 1.0],
        },
    );
    let svg = p.finish();
    assert!(
        svg.contains("<path d=\"M1.000 2.000 L9.000 8.000\" fill=\"none\" stroke=\"#ff0000\""),
        "missing path head: {svg:?}"
    );
    assert!(
        svg.contains("stroke-width=\"2.500\""),
        "missing width: {svg:?}"
    );
    assert!(
        svg.contains("stroke-linecap=\"round\""),
        "missing cap: {svg:?}"
    );
    assert!(
        svg.contains("stroke-linejoin=\"bevel\""),
        "missing join: {svg:?}"
    );
    assert!(
        svg.contains("stroke-miterlimit=\"4.000\""),
        "missing miter limit: {svg:?}"
    );
    assert!(
        svg.contains("stroke-dasharray=\"3.000 1.000\""),
        "missing dash: {svg:?}"
    );
}

#[test]
fn solid_stroke_marks_dash_none_and_does_not_leak_into_a_later_stroke() {
    let p = SvgPainter::new(100, 50);
    p.stroke_path(
        &Path::new(&seg()),
        PaintColor::Static("#ff0000"),
        &StrokeStyle {
            width: 2.5,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: &[],
        },
    );
    p.rect_stroke(rect(0, 0, 10, 10), PaintColor::Static("#00ff00"), 1.0);
    let svg = p.finish();

    assert!(
        svg.contains("stroke-dasharray=\"none\""),
        "a solid stroke must state its dash explicitly: {svg:?}"
    );
    assert_eq!(
        svg.matches("stroke-dasharray").count(),
        1,
        "only the styled path may carry a dash attribute: {svg:?}"
    );
    let rect_start = svg.find("<rect").expect("rect stroke emitted");
    let rect_end = svg[rect_start..].find("/>").unwrap() + rect_start;
    let rect = &svg[rect_start..rect_end];
    assert!(!rect.contains("dasharray"), "dash leaked to rect: {rect:?}");
    assert!(rect.contains("#00ff00"), "rect color changed: {rect:?}");
}
