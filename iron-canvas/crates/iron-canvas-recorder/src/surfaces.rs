//! Capture wrappers: [`RecordingSurface`] decorates any `Surface` and
//! [`RecordingPainter`] tees every op into a per-frame buffer while the wrapped
//! painter keeps drawing for real; [`MemSurface`] is the in-memory `Surface`
//! that drives the engine's tests.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use iron_canvas_core::geometry::CanvasMetrics;
use iron_canvas_core::geometry::pixel_rect::PixelRect;
use iron_canvas_core::geometry::prim::{Line, Point, Span};
use iron_canvas_core::layer::Surface;
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
};

use crate::ops::DrawOp;
use crate::painter::RecorderPainter;

/// In-memory `Surface` adapter. Drives `Orchestrator` for tests: every
/// drawn op is captured by the wrapped `RecorderPainter`. `resize` is a
/// no-op — the recorder has no backing pixel buffer — but `present` counts
/// each call so orchestrator tests can assert the "present iff painted"
/// contract without a real flush target.
pub struct MemSurface {
    painter: Rc<RecorderPainter>,
    presents: Cell<u32>,
}

impl MemSurface {
    pub fn new() -> Self {
        Self {
            painter: Rc::new(RecorderPainter::new()),
            presents: Cell::new(0),
        }
    }

    /// Direct handle to the recorder for op-log assertions.
    pub fn recorder(&self) -> &RecorderPainter {
        &self.painter
    }

    /// Number of `present()` flips the orchestrator has requested.
    pub fn presents(&self) -> u32 {
        self.presents.get()
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

    fn resize(&mut self, _metrics: CanvasMetrics) {}
    fn present(&self) {
        self.presents.set(self.presents.get() + 1);
    }
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
    skip_groups: Rc<RefCell<HashSet<GroupClass>>>,
    /// Depth of currently-suppressed nested `begin_group`s. While > 0, no
    /// op (including the matching `end_group`) is forked to `recorder`.
    /// Reaches 0 again when the outermost suppressed `begin_group`'s
    /// `end_group` fires.
    skip_depth: Rc<Cell<u32>>,
}

impl<P: Painter + BlitPainter> RecordingPainter<P> {
    fn should_record(&self) -> bool {
        self.enabled.get() && self.skip_depth.get() == 0
    }
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
        if self.should_record() {
            self.recorder.rect_fill(rect, color);
        }
    }

    fn fill_path(&self, points: &[Point], color: PaintColor) {
        self.inner.fill_path(points, color);
        if self.should_record() {
            self.recorder.fill_path(points, color);
        }
    }

    fn clear_rect(&self, rect: PixelRect) {
        self.inner.clear_rect(rect);
        if self.should_record() {
            self.recorder.clear_rect(rect);
        }
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.inner.rect_stroke(rect, color, width);
        if self.should_record() {
            self.recorder.rect_stroke(rect, color, width);
        }
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.inner.rect_dashed(rect, color, width);
        if self.should_record() {
            self.recorder.rect_dashed(rect, color, width);
        }
    }

    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        self.inner.stroke_line(line, color, width);
        if self.should_record() {
            self.recorder.stroke_line(line, color, width);
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.inner.stroke_hline(span, y, color, width);
        if self.should_record() {
            self.recorder.stroke_hline(span, y, color, width);
        }
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.inner.stroke_vline(x, span, color, width);
        if self.should_record() {
            self.recorder.stroke_vline(x, span, color, width);
        }
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.inner.stroke_text_hline(x1, x2, y, color, width);
        if self.should_record() {
            self.recorder.stroke_text_hline(x1, x2, y, color, width);
        }
    }

    fn push_clip(&self, rect: PixelRect) {
        self.inner.push_clip(rect);
        if self.should_record() {
            self.recorder.push_clip(rect);
        }
    }

    fn pop_clip(&self) {
        self.inner.pop_clip();
        if self.should_record() {
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
        if self.should_record() {
            self.recorder
                .fill_text(text, x, y, font_css, color, align, baseline);
        }
    }

    fn invalidate_cache(&self) {
        self.inner.invalidate_cache();
        if self.should_record() {
            self.recorder.invalidate_cache();
        }
    }

    fn reset_text_defaults(&self) {
        self.inner.reset_text_defaults();
        if self.should_record() {
            self.recorder.reset_text_defaults();
        }
    }

    fn apply_dpr_transform(&self, dpr: f64) {
        self.inner.apply_dpr_transform(dpr);
        if self.should_record() {
            self.recorder.apply_dpr_transform(dpr);
        }
    }

    fn begin_group(&self, class: GroupClass) {
        self.inner.begin_group(class);
        let depth = self.skip_depth.get();
        if depth > 0 {
            // Already inside a suppressed group — bump depth and stay
            // suppressed; the matching `end_group` will balance via the
            // same counter.
            self.skip_depth.set(depth + 1);
            return;
        }
        if !self.enabled.get() {
            return;
        }
        if self.skip_groups.borrow().contains(&class) {
            // Open a new suppression scope. depth==1 marks the outermost
            // suppressed begin; the matching end_group drops it back to 0.
            self.skip_depth.set(1);
            return;
        }
        self.recorder.begin_group(class);
    }

    fn end_group(&self) {
        self.inner.end_group();
        let depth = self.skip_depth.get();
        if depth > 0 {
            // Match the suppressed begin; do not emit the end either.
            self.skip_depth.set(depth - 1);
            return;
        }
        if !self.enabled.get() {
            return;
        }
        self.recorder.end_group();
    }
}

impl<P: Painter + BlitPainter> BlitPainter for RecordingPainter<P> {
    fn blit(&self, src: PixelRect, dst: PixelRect) {
        self.inner.blit(src, dst);
        if self.should_record() {
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
    skip_groups: Rc<RefCell<HashSet<GroupClass>>>,
    skip_depth: Rc<Cell<u32>>,
}

impl<S: Surface> RecordingSurface<S> {
    pub fn new(inner: S) -> Self {
        let inner_painter = inner.clone_painter();
        let recorder = Rc::new(RecorderPainter::new());
        let enabled = Rc::new(Cell::new(false));
        let skip_groups = Rc::new(RefCell::new(HashSet::new()));
        let skip_depth = Rc::new(Cell::new(0));
        let painter = Rc::new(RecordingPainter {
            inner: inner_painter,
            recorder: Rc::clone(&recorder),
            enabled: Rc::clone(&enabled),
            skip_groups: Rc::clone(&skip_groups),
            skip_depth: Rc::clone(&skip_depth),
        });
        Self {
            inner,
            painter,
            recorder,
            enabled,
            skip_groups,
            skip_depth,
        }
    }

    pub fn enable_recording(&self) {
        self.enabled.set(true);
    }

    /// Replace the per-surface group-suppression set. Safe to call only
    /// outside a frame — the active `skip_depth` counter assumes the set
    /// it consulted on `begin_group` is the same one the matching
    /// `end_group` sees. Mid-frame mutation would corrupt the count.
    pub fn set_skip_groups(&self, groups: HashSet<GroupClass>) {
        *self.skip_groups.borrow_mut() = groups;
        self.skip_depth.set(0);
    }

    pub fn disable_recording(&self) {
        self.enabled.set(false);
    }

    pub fn is_recording(&self) -> bool {
        self.enabled.get()
    }

    /// Clear the per-frame op buffer. Call before each paint tick.
    pub fn begin_frame(&self) {
        self.recorder.clear_ops();
    }

    /// Drain the per-frame op buffer. Call after each paint tick.
    pub fn end_frame(&self) -> Vec<DrawOp> {
        self.recorder.take_ops()
    }

    /// Borrow the inner surface — useful in tests to assert against
    /// the real backend's state independent of the recording buffer.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// This surface's recorder, so the crate's tests can assert what one frame
    /// captured between `begin_frame` and `end_frame`.
    #[cfg(test)]
    pub(crate) fn recorder(&self) -> &RecorderPainter {
        &self.recorder
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

    fn resize(&mut self, metrics: CanvasMetrics) {
        self.inner.resize(metrics);
    }

    fn present(&self) {
        self.inner.present();
    }
}
