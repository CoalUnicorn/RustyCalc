#![allow(clippy::unwrap_used)]

use iron_canvas_core::painter::TextMetrics;
use iron_canvas_core::renderer::{FontIntern, TextLine, layout_into};
use std::rc::Rc;

struct FixedWidth(f64);
impl TextMetrics for FixedWidth {
    fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
        text.len() as f64 * self.0
    }
}

#[test]
fn font_intern_returns_pointer_equal_rc_for_same_key() {
    let intern = FontIntern::new();
    let a = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    let b = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    assert!(
        Rc::ptr_eq(&a, &b),
        "identical font keys must reuse the same Rc<str>"
    );
}

#[test]
fn font_intern_distinguishes_size() {
    let intern = FontIntern::new();
    let a = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    let b = intern.get_or_build(14.0, false, false, "Arial", "Calibri");
    assert!(
        !Rc::ptr_eq(&a, &b),
        "different sizes must allocate distinct entries"
    );
    assert_ne!(
        &*a, &*b,
        "different sizes must produce different css strings"
    );
}

#[test]
fn font_intern_distinguishes_weight_and_slant() {
    let intern = FontIntern::new();
    let plain = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    let bold = intern.get_or_build(12.0, true, false, "Arial", "Calibri");
    let italic = intern.get_or_build(12.0, false, true, "Arial", "Calibri");
    assert!(!Rc::ptr_eq(&plain, &bold));
    assert!(!Rc::ptr_eq(&plain, &italic));
    assert!(!Rc::ptr_eq(&bold, &italic));
}

#[test]
fn font_intern_distinguishes_family() {
    let intern = FontIntern::new();
    let a = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    let b = intern.get_or_build(12.0, false, false, "Helvetica", "Calibri");
    assert!(!Rc::ptr_eq(&a, &b));
}

#[test]
fn font_intern_distinguishes_fallback_for_blank_families() {
    for family in ["", " \t "] {
        let intern = FontIntern::new();
        let first = intern.get_or_build(12.0, false, false, family, "Calibri");
        let second = intern.get_or_build(12.0, false, false, family, "Arial");
        assert_eq!(&*first, "12px Calibri");
        assert_eq!(&*second, "12px Arial");
        assert!(Rc::ptr_eq(
            &second,
            &intern.get_or_build(12.0, false, false, family, "Arial")
        ));
        assert!(Rc::ptr_eq(
            &first,
            &intern.get_or_build(12.0, false, false, family, "Calibri")
        ));
    }
}

#[test]
fn font_intern_ignores_unused_fallback_for_explicit_family() {
    let intern = FontIntern::new();
    let first = intern.get_or_build(12.0, false, false, "Arial", "Calibri");
    let second = intern.get_or_build(12.0, false, false, "Arial", "Helvetica");
    assert_eq!(&*first, "12px Arial");
    assert!(Rc::ptr_eq(&first, &second));
}

#[test]
fn horizontal_alignment_keeps_text_and_decoration_anchors_consistent() {
    use iron_canvas_core::geometry::pixel_rect::PixelRect;
    use iron_canvas_core::geometry::prim::Point;
    use iron_canvas_core::painter::TextAlign;
    use iron_canvas_core::renderer::RendererCore;
    use iron_canvas_core::renderer::cell::text::TextPaint;
    use iron_canvas_core::theme::CanvasTheme;
    use iron_canvas_core::{Alignment, CellKind, CellStyle, FontStyle, HAlign};
    use iron_canvas_recorder::{DrawOp, RecorderPainter};

    let cases = [
        (None, TextAlign::Start, TextAlign::End),
        (Some(HAlign::General), TextAlign::Start, TextAlign::End),
        (Some(HAlign::Left), TextAlign::Start, TextAlign::Start),
        (Some(HAlign::Fill), TextAlign::Start, TextAlign::Start),
        (Some(HAlign::Justify), TextAlign::Start, TextAlign::Start),
        (
            Some(HAlign::Distributed),
            TextAlign::Start,
            TextAlign::Start,
        ),
        (Some(HAlign::Right), TextAlign::End, TextAlign::End),
        (Some(HAlign::Center), TextAlign::Center, TextAlign::Center),
        (
            Some(HAlign::CenterContinuous),
            TextAlign::Center,
            TextAlign::Center,
        ),
    ];
    let rect = PixelRect {
        top_left: Point { x: 30, y: 40 },
        width: 100,
        height: 30,
    };
    for (horizontal, other_align, number_align) in cases {
        for kind in [
            CellKind::Text,
            CellKind::Number,
            CellKind::Logical,
            CellKind::Error,
        ] {
            let expected = if kind == CellKind::Number {
                number_align
            } else {
                other_align
            };
            let painter = Rc::new(RecorderPainter::new());
            let renderer = RendererCore::for_layer(Rc::clone(&painter));
            let style = CellStyle {
                alignment: horizontal.map(|horizontal| Alignment {
                    horizontal,
                    ..Alignment::default()
                }),
                font: FontStyle {
                    underline: true,
                    strike: true,
                    ..FontStyle::default()
                },
                ..CellStyle::default()
            };
            let mut lines = Vec::new();
            let paint =
                TextPaint::resolve_into(&renderer, rect, &style, "42".into(), kind, &mut lines)
                    .expect("short text fits the cell");
            renderer.paint_text(&paint, &CanvasTheme::light(), &lines);
            let expected_x = match expected {
                TextAlign::Start => 34.0,
                TextAlign::Center => 80.0,
                TextAlign::End => 126.0,
            };
            let ops = painter.ops();
            let anchors: Vec<_> = ops
                .iter()
                .filter_map(|op| match op {
                    DrawOp::FillText { x, align, .. } => Some((*x, *align)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                anchors,
                [(expected_x, expected)],
                "{horizontal:?} / {kind:?}"
            );
            let width = lines[0].width;
            let expected_left = match expected {
                TextAlign::Start => expected_x,
                TextAlign::Center => expected_x - width / 2.0,
                TextAlign::End => expected_x - width,
            };
            let decorations: Vec<_> = ops
                .iter()
                .filter_map(|op| match op {
                    DrawOp::StrokeTextHLine { x1, x2, .. } => Some((*x1, *x2)),
                    _ => None,
                })
                .collect();
            assert_eq!(decorations.len(), 2);
            for (left, right) in decorations {
                assert!((left - expected_left).abs() < 1e-9);
                assert!((right - expected_left - width).abs() < 1e-9);
            }
        }
    }
}

// -----
// layout_into: TextLine String capacity reuse
//
// The per-cell text path is hot. `lines` and `wrap_buf` are parked on
// `FrameCache` so the second cell of a frame should not allocate. These tests
// pin that contract: slot 0's `String` heap buffer survives across calls, and
// stale lines from a longer prior cell are truncated rather than rendered.
// -----

#[test]
fn layout_into_reuses_text_line_string_buffer() {
    let metrics = FixedWidth(8.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut wrap_buf = String::new();

    layout_into(
        &metrics,
        "12px sans",
        "hello\nworld",
        false,
        1000.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    assert_eq!(lines.len(), 2);
    let ptr_first = lines[0].text.as_ptr();
    let cap_first = lines[0].text.capacity();
    assert!(cap_first >= 5);

    layout_into(
        &metrics,
        "12px sans",
        "hi",
        false,
        1000.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    assert_eq!(lines.len(), 1);
    assert_eq!(&lines[0].text, "hi");

    assert!(
        lines[0].text.capacity() >= cap_first,
        "slot 0 capacity must not shrink across calls"
    );
    assert_eq!(
        ptr_first,
        lines[0].text.as_ptr(),
        "slot 0 String buffer must be reused, not reallocated"
    );
}

#[test]
fn layout_into_truncates_stale_lines_from_previous_call() {
    let metrics = FixedWidth(8.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut wrap_buf = String::new();

    layout_into(
        &metrics,
        "12px",
        "a\nb\nc\nd",
        false,
        1000.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    assert_eq!(lines.len(), 4);

    layout_into(
        &metrics,
        "12px",
        "x",
        false,
        1000.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    assert_eq!(
        lines.len(),
        1,
        "stale lines from previous longer call must be truncated"
    );
    assert_eq!(&lines[0].text, "x");
}

#[test]
fn layout_into_wrap_buf_buffer_persists() {
    let metrics = FixedWidth(8.0);
    let mut lines: Vec<TextLine> = Vec::new();
    let mut wrap_buf = String::new();

    layout_into(
        &metrics,
        "12px",
        "the quick brown fox jumps",
        true,
        80.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    let cap_first = wrap_buf.capacity();
    let ptr_first = wrap_buf.as_ptr();
    assert!(cap_first > 0, "wrap path must populate wrap_buf");

    layout_into(
        &metrics,
        "12px",
        "x",
        true,
        80.0,
        8.0,
        &mut lines,
        &mut wrap_buf,
    );
    assert!(
        wrap_buf.capacity() >= cap_first,
        "wrap_buf capacity must not shrink"
    );
    assert_eq!(
        ptr_first,
        wrap_buf.as_ptr(),
        "wrap_buf heap buffer must be reused"
    );
}
