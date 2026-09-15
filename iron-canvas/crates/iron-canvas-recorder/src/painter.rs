//! [`RecorderPainter`] — a `Painter` / `BlitPainter` that appends every call to
//! an op log instead of rasterizing.
//!
//! `measure_text_width` returns a deterministic estimate (core's
//! `approx_text_width` over the parsed font size). Tests that assert real
//! text-wrap behavior against browser metrics still need a wasm-bindgen-test
//! harness.

use std::cell::{Cell, RefCell};

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    approx_text_width, parse_font_size_px,
};

use crate::ops::DrawOp;

#[derive(Default)]
pub struct RecorderPainter {
    ops: RefCell<Vec<DrawOp>>,
    clip_depth: Cell<u32>,
    group_depth: Cell<u32>,
}

impl RecorderPainter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ops(&self) -> std::cell::Ref<'_, Vec<DrawOp>> {
        self.ops.borrow()
    }

    pub fn into_ops(self) -> Vec<DrawOp> {
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

    /// Drop every buffered op. The frame boundary `RecordingSurface` drives.
    pub(crate) fn clear_ops(&self) {
        self.ops.borrow_mut().clear();
    }

    /// Take the buffered ops, leaving the buffer empty. The other half of the
    /// frame boundary `RecordingSurface` drives.
    pub(crate) fn take_ops(&self) -> Vec<DrawOp> {
        std::mem::take(&mut *self.ops.borrow_mut())
    }
}

impl TextMetrics for RecorderPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        approx_text_width(parse_font_size_px(font_css), text)
    }
}

impl Painter for RecorderPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.push(DrawOp::RectFill {
            rect,
            color: color.as_str().to_string(),
        });
    }

    fn fill_path(&self, points: &[Point], color: PaintColor) {
        self.push(DrawOp::FillPath {
            points: points.to_vec(),
            color: color.as_str().to_string(),
        });
    }

    fn clear_rect(&self, rect: PixelRect) {
        self.push(DrawOp::ClearRect { rect });
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

    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        self.push(DrawOp::StrokeLine {
            line,
            color: color.as_str().to_string(),
            width,
        });
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.push(DrawOp::StrokeHLine {
            span,
            y,
            color: color.as_str().to_string(),
            width,
        });
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.push(DrawOp::StrokeVLine {
            x,
            span,
            color: color.as_str().to_string(),
            width,
        });
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.push(DrawOp::StrokeTextHLine {
            x1,
            x2,
            y,
            color: color.as_str().to_string(),
            width,
        });
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

    fn apply_dpr_transform(&self, dpr: f64) {
        self.push(DrawOp::ApplyDprTransform { dpr });
    }

    fn begin_group(&self, class: GroupClass) {
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
}

impl BlitPainter for RecorderPainter {
    fn blit(&self, src: PixelRect, dst: PixelRect) {
        self.push(DrawOp::Blit { src, dst });
    }
}
