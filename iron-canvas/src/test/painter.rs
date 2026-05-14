//! Test-only `Painter` impl that records every draw call as a `DrawOp`.
//!
//! `measure_text_width` returns a deterministic estimate (chars × font_size
//! × CHAR_WIDTH_FACTOR). Tests asserting real text-wrap behavior against
//! browser metrics still need a wasm-bindgen-test harness.

use std::cell::{Cell, RefCell};

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Line, Span};
use crate::painter::{PaintColor, Painter, Sealed, TextAlign, TextBaseline, TextMetrics};

/// Per-char width factor as a fraction of font size. Matches the
/// approx-char-width fallback in `text_paint.rs::layout_into` so wrap math
/// in tests stays internally consistent with the layout's own fallback.
const CHAR_WIDTH_FACTOR: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DrawOp {
    RectFill {
        rect: PixelRect,
        color: String,
    },
    RectStroke {
        rect: PixelRect,
        color: String,
        width: f64,
    },
    RectDashed {
        rect: PixelRect,
        color: String,
        width: f64,
    },
    StrokeLine {
        line: Line,
    },
    StrokeHLine {
        span: Span,
        y: f64,
    },
    StrokeVLine {
        x: f64,
        span: Span,
    },
    StrokeTextHLine {
        x1: f64,
        x2: f64,
        y: f64,
    },
    PushClip {
        rect: PixelRect,
    },
    PopClip,
    FillText {
        text: String,
        x: f64,
        y: f64,
        font_css: String,
        color: String,
        align: TextAlign,
        baseline: TextBaseline,
    },
    InvalidateCache,
    ResetTextDefaults,
    ApplyDprTransform {
        dpr: i32,
    },
    BeginGroup {
        class: &'static str,
    },
    EndGroup,
    Blit {
        src: PixelRect,
        dst: PixelRect,
    },
}

#[derive(Default)]
pub(crate) struct RecorderPainter {
    ops: RefCell<Vec<DrawOp>>,
    clip_depth: Cell<u32>,
    group_depth: Cell<u32>,
}

impl RecorderPainter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    #[allow(dead_code)]
    pub(crate) fn ops(&self) -> std::cell::Ref<'_, Vec<DrawOp>> {
        self.ops.borrow()
    }

    pub(crate) fn into_ops(self) -> Vec<DrawOp> {
        debug_assert_eq!(
            self.clip_depth.get(),
            0,
            "RecorderPainter dropped with unbalanced push_clip/pop_clip",
        );
        debug_assert_eq!(
            self.group_depth.get(),
            0,
            "RecorderPainter dropped with unbalanced begin_group/end_group",
        );
        self.ops.into_inner()
    }

    fn push(&self, op: DrawOp) {
        self.ops.borrow_mut().push(op);
    }
}

impl Sealed for RecorderPainter {}

impl TextMetrics for RecorderPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        let font_size_px = font_css
            .split_whitespace()
            .find_map(|tok| tok.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()))
            .unwrap_or(12.0);
        text.chars().count() as f64 * font_size_px * CHAR_WIDTH_FACTOR
    }
}

impl Painter for RecorderPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.push(DrawOp::RectFill {
            rect,
            color: color.as_str().to_string(),
        });
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.push(DrawOp::RectStroke {
            rect,
            color: color.as_str().to_string(),
            width,
        });
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.push(DrawOp::RectDashed {
            rect,
            color: color.as_str().to_string(),
            width,
        });
    }

    fn stroke_line(&self, line: Line, _color: PaintColor, _width: f64) {
        self.push(DrawOp::StrokeLine { line });
    }

    fn stroke_hline(&self, span: Span, y: f64, _color: PaintColor, _width: f64) {
        self.push(DrawOp::StrokeHLine { span, y });
    }

    fn stroke_vline(&self, x: f64, span: Span, _color: PaintColor, _width: f64) {
        self.push(DrawOp::StrokeVLine { x, span });
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, _color: PaintColor, _width: f64) {
        self.push(DrawOp::StrokeTextHLine { x1, x2, y });
    }

    fn push_clip(&self, rect: PixelRect) {
        self.push(DrawOp::PushClip { rect });
        self.clip_depth.set(self.clip_depth.get() + 1);
    }

    fn pop_clip(&self) {
        debug_assert!(
            self.clip_depth.get() > 0,
            "RecorderPainter pop_clip without matching push_clip",
        );
        self.clip_depth.set(self.clip_depth.get() - 1);
        self.push(DrawOp::PopClip);
    }

    fn fill_text(
        &self,
        text: &str,
        x: f64,
        y: f64,
        font_css: PaintColor,
        color: PaintColor,
        align: TextAlign,
        baseline: TextBaseline,
    ) {
        self.push(DrawOp::FillText {
            text: text.to_string(),
            x,
            y,
            font_css: font_css.as_str().to_string(),
            color: color.as_str().to_string(),
            align,
            baseline,
        });
    }

    fn invalidate_cache(&self) {
        self.push(DrawOp::InvalidateCache);
    }

    fn reset_text_defaults(&self) {
        self.push(DrawOp::ResetTextDefaults);
    }

    fn apply_dpr_transform(&self, dpr: i32) {
        self.push(DrawOp::ApplyDprTransform { dpr });
    }

    fn begin_group(&self, class: &'static str) {
        self.push(DrawOp::BeginGroup { class });
        self.group_depth.set(self.group_depth.get() + 1);
    }

    fn end_group(&self) {
        debug_assert!(
            self.group_depth.get() > 0,
            "RecorderPainter end_group without matching begin_group",
        );
        self.group_depth.set(self.group_depth.get() - 1);
        self.push(DrawOp::EndGroup);
    }

    fn supports_blit(&self) -> bool {
        true
    }

    fn blit(&self, src: PixelRect, dst: PixelRect) {
        self.push(DrawOp::Blit { src, dst });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::prim::Point;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> PixelRect {
        PixelRect {
            top_left: Point {
                x: x as i32,
                y: y as i32,
            },
            width: w as i32,
            height: h as i32,
        }
    }

    #[test]
    fn rect_fill_records_op() {
        let p = RecorderPainter::new();
        p.rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#ff0000"));
        let ops = p.into_ops();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            ops[0],
            DrawOp::RectFill { ref color, .. } if color == "#ff0000"
        ));
    }

    #[test]
    fn push_pop_clip_balances_depth() {
        let p = RecorderPainter::new();
        p.push_clip(rect(0.0, 0.0, 10.0, 10.0));
        p.pop_clip();
        let ops = p.into_ops();
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], DrawOp::PushClip { .. }));
        assert!(matches!(ops[1], DrawOp::PopClip));
    }

    #[test]
    fn blit_records_op_and_supports_blit_is_true() {
        let p = RecorderPainter::new();
        assert!(p.supports_blit(), "Recorder backend opts in to blit");
        let src = rect(0.0, 20.0, 100.0, 200.0);
        let dst = rect(0.0, 0.0, 100.0, 200.0);
        p.blit(src, dst);
        let ops = p.into_ops();
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            ops[0],
            DrawOp::Blit { src: s, dst: d } if s == src && d == dst
        ));
    }

    #[test]
    fn measure_text_width_parses_font_size_from_css() {
        let p = RecorderPainter::new();
        // 5 chars × 16px × 1.0
        assert_eq!(p.measure_text_width("hello", "16px sans-serif"), 80.0);
        // bold prefix shouldn't break parse
        assert_eq!(p.measure_text_width("hi", "bold 12px sans-serif"), 24.0);
        // missing size falls back to 12px default
        assert_eq!(p.measure_text_width("ab", "no-size"), 24.0);
    }
}
