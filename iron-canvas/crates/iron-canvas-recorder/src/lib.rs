//! Test-only `Painter` impl that records every draw call as a `DrawOp`.
//!
//! `measure_text_width` returns a deterministic estimate (chars × font_size
//! × CHAR_WIDTH_FACTOR). Tests asserting real text-wrap behavior against
//! browser metrics still need a wasm-bindgen-test harness.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Span};
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
};

use serde::{Deserialize, Serialize};

pub mod recording;

/// Per-char width factor as a fraction of font size. Matches the
/// approx-char-width fallback in `text_paint.rs::layout_into` so wrap math
/// in tests stays internally consistent with the layout's own fallback.
const CHAR_WIDTH_FACTOR: f64 = 1.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DrawOp {
    RectFill {
        rect: PixelRect,
        color: String,
    },
    ClearRect {
        rect: PixelRect,
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
        color: String,
        width: f64,
    },
    StrokeHLine {
        span: Span,
        y: f64,
        color: String,
        width: f64,
    },
    StrokeVLine {
        x: f64,
        span: Span,
        color: String,
        width: f64,
    },
    StrokeTextHLine {
        x1: f64,
        x2: f64,
        y: f64,
        color: String,
        width: f64,
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
        class: GroupClass,
    },
    EndGroup,
    Blit {
        src: PixelRect,
        dst: PixelRect,
    },
}

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

    #[allow(dead_code)]
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
}

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

    fn apply_dpr_transform(&self, dpr: i32) {
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

/// Replay a captured op log onto any `BlitPainter`. Debug-visualizer
/// path — e.g. record once via `RecorderPainter`, then replay onto
/// `SvgPainter` for a golden artifact, or onto a second `RecorderPainter`
/// to round-trip the log. Calls `target.invalidate_cache()` once before
/// dispatch so the target's ctx-state cache (`last_fill` / `last_stroke` /
/// `last_font` / `last_line_width`) is not desync'd against the replayed
/// stream.
///
/// Recorded color / font strings are owned `String`s — replay routes them
/// through `PaintColor::Borrowed`, which falls back to the content-eq
/// cache on the target (no ptr-eq fast path). Built-in-theme ptr-eq is
/// only available on the original render pass.
pub fn replay<P: BlitPainter>(target: &P, ops: &[DrawOp]) {
    target.invalidate_cache();
    for op in ops {
        match op {
            DrawOp::RectFill { rect, color } => {
                target.rect_fill(*rect, PaintColor::Borrowed(color));
            }
            DrawOp::ClearRect { rect } => target.clear_rect(*rect),
            DrawOp::RectStroke { rect, color, width } => {
                target.rect_stroke(*rect, PaintColor::Borrowed(color), *width);
            }
            DrawOp::RectDashed { rect, color, width } => {
                target.rect_dashed(*rect, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeLine { line, color, width } => {
                target.stroke_line(*line, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeHLine {
                span,
                y,
                color,
                width,
            } => {
                target.stroke_hline(*span, *y, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeVLine {
                x,
                span,
                color,
                width,
            } => {
                target.stroke_vline(*x, *span, PaintColor::Borrowed(color), *width);
            }
            DrawOp::StrokeTextHLine {
                x1,
                x2,
                y,
                color,
                width,
            } => {
                target.stroke_text_hline(*x1, *x2, *y, PaintColor::Borrowed(color), *width);
            }
            DrawOp::PushClip { rect } => target.push_clip(*rect),
            DrawOp::PopClip => target.pop_clip(),
            DrawOp::FillText {
                text,
                x,
                y,
                font_css,
                color,
                align,
                baseline,
            } => target.fill_text(
                text,
                *x,
                *y,
                PaintColor::Borrowed(font_css),
                PaintColor::Borrowed(color),
                *align,
                *baseline,
            ),
            DrawOp::InvalidateCache => target.invalidate_cache(),
            DrawOp::ResetTextDefaults => target.reset_text_defaults(),
            DrawOp::ApplyDprTransform { dpr } => target.apply_dpr_transform(*dpr),
            DrawOp::BeginGroup { class } => target.begin_group(*class),
            DrawOp::EndGroup => target.end_group(),
            DrawOp::Blit { src, dst } => target.blit(*src, *dst),
        }
    }
}

/// In-memory `Surface` adapter. Drives `Orchestrator` for tests: every
/// drawn op is captured by the wrapped `RecorderPainter`. `resize` /
/// `present` are no-ops — the recorder has no backing pixel buffer.
pub struct MemSurface {
    painter: Rc<RecorderPainter>,
}

impl MemSurface {
    pub fn new() -> Self {
        Self {
            painter: Rc::new(RecorderPainter::new()),
        }
    }

    /// Direct handle to the recorder for op-log assertions.
    pub fn recorder(&self) -> &RecorderPainter {
        &self.painter
    }
}

impl Default for MemSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl Surface for MemSurface {
    type P = RecorderPainter;

    fn painter(&self) -> &RecorderPainter {
        self.painter.as_ref()
    }

    fn clone_painter(&self) -> Rc<RecorderPainter> {
        Rc::clone(&self.painter)
    }

    fn resize(&mut self, _css: CanvasSize, _dpr: i32) {}
    fn present(&self) {}
}

/// Painter wrapper that forwards every op to an inner painter and, only
/// when recording is enabled, also forks the op into a shared
/// `RecorderPainter`. The forward leg is unconditional — production
/// rendering still drives the real backend — so toggling recording
/// off costs exactly one branch per op (no allocation, no Vec push).
///
/// Built by `RecordingSurface`; not constructed directly by callers.
pub struct RecordingPainter<P: Painter + BlitPainter> {
    inner: Rc<P>,
    recorder: Rc<RecorderPainter>,
    enabled: Rc<Cell<bool>>,
}

impl<P: Painter + BlitPainter> TextMetrics for RecordingPainter<P> {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        // Query, not an op — go to inner for the real measurement.
        // Recorder's approximation must not bleed into paint geometry.
        self.inner.measure_text_width(text, font_css)
    }
}

impl<P: Painter + BlitPainter> Painter for RecordingPainter<P> {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.inner.rect_fill(rect, color);
        if self.enabled.get() {
            self.recorder.rect_fill(rect, color);
        }
    }

    fn clear_rect(&self, rect: PixelRect) {
        self.inner.clear_rect(rect);
        if self.enabled.get() {
            self.recorder.clear_rect(rect);
        }
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.inner.rect_stroke(rect, color, width);
        if self.enabled.get() {
            self.recorder.rect_stroke(rect, color, width);
        }
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.inner.rect_dashed(rect, color, width);
        if self.enabled.get() {
            self.recorder.rect_dashed(rect, color, width);
        }
    }

    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        self.inner.stroke_line(line, color, width);
        if self.enabled.get() {
            self.recorder.stroke_line(line, color, width);
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.inner.stroke_hline(span, y, color, width);
        if self.enabled.get() {
            self.recorder.stroke_hline(span, y, color, width);
        }
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.inner.stroke_vline(x, span, color, width);
        if self.enabled.get() {
            self.recorder.stroke_vline(x, span, color, width);
        }
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.inner.stroke_text_hline(x1, x2, y, color, width);
        if self.enabled.get() {
            self.recorder.stroke_text_hline(x1, x2, y, color, width);
        }
    }

    fn push_clip(&self, rect: PixelRect) {
        self.inner.push_clip(rect);
        if self.enabled.get() {
            self.recorder.push_clip(rect);
        }
    }

    fn pop_clip(&self) {
        self.inner.pop_clip();
        if self.enabled.get() {
            self.recorder.pop_clip();
        }
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
        self.inner
            .fill_text(text, x, y, font_css, color, align, baseline);
        if self.enabled.get() {
            self.recorder
                .fill_text(text, x, y, font_css, color, align, baseline);
        }
    }

    fn invalidate_cache(&self) {
        self.inner.invalidate_cache();
        if self.enabled.get() {
            self.recorder.invalidate_cache();
        }
    }

    fn reset_text_defaults(&self) {
        self.inner.reset_text_defaults();
        if self.enabled.get() {
            self.recorder.reset_text_defaults();
        }
    }

    fn apply_dpr_transform(&self, dpr: i32) {
        self.inner.apply_dpr_transform(dpr);
        if self.enabled.get() {
            self.recorder.apply_dpr_transform(dpr);
        }
    }

    fn begin_group(&self, class: GroupClass) {
        self.inner.begin_group(class);
        if self.enabled.get() {
            self.recorder.begin_group(class);
        }
    }

    fn end_group(&self) {
        self.inner.end_group();
        if self.enabled.get() {
            self.recorder.end_group();
        }
    }
}

impl<P: Painter + BlitPainter> BlitPainter for RecordingPainter<P> {
    fn blit(&self, src: PixelRect, dst: PixelRect) {
        self.inner.blit(src, dst);
        if self.enabled.get() {
            self.recorder.blit(src, dst);
        }
    }
}

/// `Surface` decorator that wraps an inner `Surface` and forks every
/// `Painter` call into a per-frame op buffer when recording is enabled.
///
/// Frame boundary contract: callers must call `begin_frame()` before
/// each paint tick and `end_frame()` after — the buffer is per-frame,
/// not cumulative. `enable_recording` / `disable_recording` flip the
/// fork at the painter level; flipping mid-frame is **not supported**
/// (could land orphan `push_clip` without `pop_clip` in the buffer and
/// trip `RecorderPainter`'s balance asserts at drain time).
pub struct RecordingSurface<S: Surface> {
    inner: S,
    painter: Rc<RecordingPainter<S::P>>,
    recorder: Rc<RecorderPainter>,
    enabled: Rc<Cell<bool>>,
}

impl<S: Surface> RecordingSurface<S> {
    pub fn new(inner: S) -> Self {
        let inner_painter = inner.clone_painter();
        let recorder = Rc::new(RecorderPainter::new());
        let enabled = Rc::new(Cell::new(false));
        let painter = Rc::new(RecordingPainter {
            inner: inner_painter,
            recorder: Rc::clone(&recorder),
            enabled: Rc::clone(&enabled),
        });
        Self {
            inner,
            painter,
            recorder,
            enabled,
        }
    }

    pub fn enable_recording(&self) {
        self.enabled.set(true);
    }

    pub fn disable_recording(&self) {
        self.enabled.set(false);
    }

    pub fn is_recording(&self) -> bool {
        self.enabled.get()
    }

    /// Clear the per-frame op buffer. Call before each paint tick.
    pub fn begin_frame(&self) {
        self.recorder.ops.borrow_mut().clear();
    }

    /// Drain the per-frame op buffer. Call after each paint tick.
    pub fn end_frame(&self) -> Vec<DrawOp> {
        std::mem::take(&mut *self.recorder.ops.borrow_mut())
    }

    /// Borrow the inner surface — useful in tests to assert against
    /// the real backend's state independent of the recording buffer.
    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: Surface> Surface for RecordingSurface<S> {
    type P = RecordingPainter<S::P>;

    fn painter(&self) -> &RecordingPainter<S::P> {
        self.painter.as_ref()
    }

    fn clone_painter(&self) -> Rc<RecordingPainter<S::P>> {
        Rc::clone(&self.painter)
    }

    fn resize(&mut self, css: CanvasSize, dpr: i32) {
        self.inner.resize(css, dpr);
    }

    fn present(&self) {
        self.inner.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::geometry::prim::Point;

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
    fn blit_records_op() {
        let p = RecorderPainter::new();
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
    fn replay_round_trips_op_log() {
        // Emit one of every op into the source, replay into a sink,
        // and assert the sink's log equals the source's (modulo the
        // leading `InvalidateCache` that `replay` always prepends).
        let src = RecorderPainter::new();
        let r = rect(0.0, 0.0, 10.0, 10.0);
        src.rect_fill(r, PaintColor::Static("#ff0000"));
        src.clear_rect(r);
        src.rect_stroke(r, PaintColor::Static("#00ff00"), 1.0);
        src.rect_dashed(r, PaintColor::Static("#0000ff"), 2.0);
        src.stroke_line(
            Line::H {
                span: Span { from: 0, to: 10 },
                y: 5,
            },
            PaintColor::Static("#111"),
            1.0,
        );
        src.stroke_hline(
            Span { from: 0, to: 10 },
            5.0,
            PaintColor::Static("#222"),
            1.0,
        );
        src.stroke_vline(
            5.0,
            Span { from: 0, to: 10 },
            PaintColor::Static("#333"),
            1.0,
        );
        src.stroke_text_hline(0.0, 10.0, 5.0, PaintColor::Static("#444"), 1.5);
        src.push_clip(r);
        src.fill_text(
            "hi",
            1.0,
            2.0,
            PaintColor::Static("12px sans"),
            PaintColor::Static("#555"),
            TextAlign::Start,
            TextBaseline::Top,
        );
        src.pop_clip();
        src.invalidate_cache();
        src.reset_text_defaults();
        src.apply_dpr_transform(2);
        src.begin_group(GroupClass::Grid);
        src.end_group();
        src.blit(r, r);

        let ops = src.into_ops();

        let sink = RecorderPainter::new();
        super::replay(&sink, &ops);
        let replayed = sink.into_ops();

        // replay prepends one InvalidateCache; assert the tail matches.
        assert!(matches!(replayed[0], DrawOp::InvalidateCache));
        assert_eq!(&replayed[1..], &ops[..]);
    }

    #[test]
    fn recording_painter_forks_when_enabled() {
        let surface = RecordingSurface::new(MemSurface::new());
        surface.enable_recording();
        surface.begin_frame();
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#fff"));
        let ops = surface.end_frame();
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], DrawOp::RectFill { .. }));
    }

    #[test]
    fn recording_painter_no_fork_when_disabled() {
        let surface = RecordingSurface::new(MemSurface::new());
        // recording defaults to off
        surface.begin_frame();
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#fff"));
        let ops = surface.end_frame();
        assert!(ops.is_empty(), "disabled recording should capture nothing");
    }

    #[test]
    fn recording_painter_forwards_to_inner_always() {
        // Whether enabled or not, the inner painter must receive every op
        // — production rendering depends on it.
        let surface = RecordingSurface::new(MemSurface::new());
        // disabled
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#aaa"));
        // enabled
        surface.enable_recording();
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 20.0, 20.0), PaintColor::Static("#bbb"));
        // Inner MemSurface's RecorderPainter sees BOTH ops.
        assert_eq!(surface.inner().recorder().ops().len(), 2);
    }

    #[test]
    fn begin_frame_clears_buffer() {
        let surface = RecordingSurface::new(MemSurface::new());
        surface.enable_recording();
        surface.begin_frame();
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#fff"));
        assert_eq!(surface.recorder.ops.borrow().len(), 1);
        surface.begin_frame();
        assert!(
            surface.recorder.ops.borrow().is_empty(),
            "begin_frame should discard prior frame's ops",
        );
    }

    #[test]
    fn end_frame_drains_buffer() {
        let surface = RecordingSurface::new(MemSurface::new());
        surface.enable_recording();
        surface.begin_frame();
        surface
            .painter()
            .rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#fff"));
        let ops = surface.end_frame();
        assert_eq!(ops.len(), 1);
        assert!(
            surface.recorder.ops.borrow().is_empty(),
            "end_frame should leave the buffer empty",
        );
    }

    #[test]
    fn surface_painter_round_trip() {
        // Drive the RecordingSurface through one painter's worth of ops,
        // end_frame the captured stream, replay into a fresh recorder,
        // and assert byte-equal (modulo the leading InvalidateCache
        // that `replay` prepends).
        let surface = RecordingSurface::new(MemSurface::new());
        surface.enable_recording();
        surface.begin_frame();
        let p = surface.painter();
        p.rect_fill(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#fff"));
        p.rect_stroke(rect(0.0, 0.0, 10.0, 10.0), PaintColor::Static("#000"), 1.0);
        p.push_clip(rect(0.0, 0.0, 5.0, 5.0));
        p.fill_text(
            "hi",
            1.0,
            2.0,
            PaintColor::Static("12px sans"),
            PaintColor::Static("#000"),
            TextAlign::Start,
            TextBaseline::Top,
        );
        p.pop_clip();
        p.blit(rect(0.0, 20.0, 10.0, 10.0), rect(0.0, 0.0, 10.0, 10.0));
        let captured = surface.end_frame();

        let sink = RecorderPainter::new();
        crate::replay(&sink, &captured);
        let replayed = sink.into_ops();

        assert!(matches!(replayed[0], DrawOp::InvalidateCache));
        assert_eq!(&replayed[1..], &captured[..]);
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
