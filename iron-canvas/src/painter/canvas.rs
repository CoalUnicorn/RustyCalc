//! Canvas 2D backend for `Painter`.
//!
//! Owns the `CanvasRenderingContext2d` plus the per-frame state cache
//! (`Cell<CachedColor>` for fill/stroke/font, `Cell<f64>` for line_width).
//! The cache is a Canvas2D-only optimization — it lives here, not in the
//! trait, because Recorder/SVG backends do not need it.

use std::cell::Cell;

use web_sys::{js_sys, CanvasRenderingContext2d};

use super::{PaintColor, Painter, Sealed, TextAlign, TextBaseline, TextMetrics};
use crate::geometry::{
    pixel_rect::PixelRect,
    prim::{Line, Span},
};

/// Standard border line width (1px in CSS pixels).
pub(crate) const STANDARD_BORDER_WIDTH: f64 = 1.0;

/// Cached color/font value. `Static` is the zero-alloc fast path: when the
/// renderer pushed a `&'static str` (theme color, `HEADER_FONT`), we keep the
/// reference and ptr-eq it on the next call. `Owned` carries a per-frame
/// custom color that originated as a non-static `&str`. `Empty` is the
/// initial / post-clip state — always misses so the next paint re-binds the
/// ctx.
#[derive(Default, Clone)]
pub(crate) enum CachedColor {
    #[default]
    Empty,
    Static(&'static str),
    Owned(String),
}

impl CachedColor {
    /// True when the next paint can skip the `ctx.set_*` round-trip.
    ///
    /// `Static-Static` is the zero-cost path — same `&'static str` literal
    /// pointer means same content. Falling back to content-eq across the
    /// other variants keeps us correct when a `Borrowed` color happens to
    /// equal a previously cached `Static`, or vice-versa.
    pub fn matches(&self, other: PaintColor) -> bool {
        match (self, other) {
            (CachedColor::Empty, _) => false,
            (CachedColor::Static(a), PaintColor::Static(b)) => std::ptr::eq(a.as_ptr(), b.as_ptr()),
            (CachedColor::Static(a), PaintColor::Borrowed(b)) => *a == b,
            (CachedColor::Owned(a), other) => a == other.as_str(),
        }
    }
}

pub(crate) struct CanvasPainter {
    pub ctx: CanvasRenderingContext2d,
    pub last_fill: Cell<CachedColor>,
    pub last_stroke: Cell<CachedColor>,
    pub last_font: Cell<CachedColor>,
    pub last_line_width: Cell<f64>,
    pub dash_pattern: js_sys::Array,
    pub dash_empty: js_sys::Array,
    pub clip_depth: Cell<u32>,
}

impl CanvasPainter {
    pub(crate) fn new(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            last_fill: Cell::new(CachedColor::Empty),
            last_stroke: Cell::new(CachedColor::Empty),
            last_font: Cell::new(CachedColor::Empty),
            last_line_width: Cell::new(0.0),
            dash_pattern: js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into()),
            dash_empty: js_sys::Array::new(),
            clip_depth: Cell::new(0),
        }
    }

    /// Direct ctx access for the layer wrappers' own clear/fill paths
    /// (`GridLayer::paint`, `OverlayLayer::paint`). Renderer code never
    /// calls this; it routes through the `Painter` surface instead.
    pub(crate) fn ctx(&self) -> &CanvasRenderingContext2d {
        &self.ctx
    }

    pub(crate) fn set_fill_cached(&self, color: PaintColor) {
        let prev = self.last_fill.take();
        if prev.matches(color) {
            self.last_fill.set(prev);
            return;
        }
        self.ctx.set_fill_style_str(color.as_str());
        self.last_fill.set(into_cached(color));
    }

    fn set_stroke_cached(&self, color: PaintColor) {
        let prev = self.last_stroke.take();
        if prev.matches(color) {
            self.last_stroke.set(prev);
            return;
        }
        self.ctx.set_stroke_style_str(color.as_str());
        self.last_stroke.set(into_cached(color));
    }

    pub(crate) fn set_font_cached(&self, font: PaintColor) {
        let prev = self.last_font.take();
        if prev.matches(font) {
            self.last_font.set(prev);
            return;
        }
        self.ctx.set_font(font.as_str());
        self.last_font.set(into_cached(font));
    }

    pub(crate) fn set_line_width_cached(&self, width: f64) {
        if (self.last_line_width.get() - width).abs() > f64::EPSILON {
            self.ctx.set_line_width(width);
            self.last_line_width.set(width);
        }
    }

    pub(crate) fn with_stroke_width<F: FnOnce(&Self)>(&self, width: f64, f: F) {
        self.set_line_width_cached(width);
        f(self);
        self.set_line_width_cached(STANDARD_BORDER_WIDTH);
    }
}

/// Branchless mapping from the call-site's `PaintColor` to the right
/// `CachedColor` variant. Static stays zero-alloc; Borrowed pays one
/// `to_string()` to own the comparison key for next-call content-eq.
#[inline]
fn into_cached(color: PaintColor<'_>) -> CachedColor {
    match color {
        PaintColor::Static(s) => CachedColor::Static(s),
        PaintColor::Borrowed(s) => CachedColor::Owned(s.to_string()),
    }
}

impl Drop for CanvasPainter {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.clip_depth.get(),
            0,
            "CanvasPainter dropped with unbalanced push_clip/pop_clip"
        );
    }
}

impl Sealed for CanvasPainter {}

impl TextMetrics for CanvasPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        // Metrics callers (FontIntern + HEADER_FONT only feed `fill_text`)
        // pass an `&str` here. Treat as Borrowed — the cache will still hit
        // against a previously cached Static via content-eq if it was the
        // same literal.
        self.set_font_cached(PaintColor::Borrowed(font_css));
        self.ctx
            .measure_text(text)
            .map(|m| m.width())
            .unwrap_or_else(|_| text.chars().count() as f64 * 6.0)
    }
}

impl Painter for CanvasPainter {
    fn rect_fill(&self, rect: PixelRect, color: PaintColor) {
        self.set_fill_cached(color);
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.fill_rect(x, y, w, h);
    }

    fn rect_stroke(&self, rect: PixelRect, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        let (x, y, w, h) = rect.as_f64_tuple();
        self.with_stroke_width(width, |this| {
            this.ctx.stroke_rect(x, y, w, h);
        });
    }

    fn rect_dashed(&self, rect: PixelRect, color: PaintColor, width: f64) {
        let _ = self.ctx.set_line_dash(&self.dash_pattern);
        self.rect_stroke(rect, color, width);
        let _ = self.ctx.set_line_dash(&self.dash_empty);
    }

    /// Stroke an axis-aligned `Line`. Dispatches on the enum variant so the
    /// caller doesn't pick `stroke_hline` vs `stroke_vline` manually.
    fn stroke_line(&self, line: Line, color: PaintColor, width: f64) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y), color, width),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span, color, width),
        }
    }

    fn stroke_hline(&self, span: Span, y: f64, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(f64::from(span.from), y);
        self.ctx.line_to(f64::from(span.to), y);
        self.ctx.stroke();
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(x, f64::from(span.from));
        self.ctx.line_to(x, f64::from(span.to));
        self.ctx.stroke();
    }

    fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64, color: PaintColor, width: f64) {
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(x1, y);
        self.ctx.line_to(x2, y);
        self.ctx.stroke();
    }

    fn push_clip(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(x, y, w, h);
        self.ctx.clip();
        self.clip_depth.set(self.clip_depth.get() + 1);
    }

    fn pop_clip(&self) {
        debug_assert!(
            self.clip_depth.get() > 0,
            "pop_clip without matching push_clip"
        );
        self.ctx.restore();
        self.clip_depth.set(self.clip_depth.get() - 1);
        // restore() resets fill/stroke/font/lineWidth/dash — invalidate cache
        self.last_fill.set(CachedColor::Empty);
        self.last_stroke.set(CachedColor::Empty);
        self.last_font.set(CachedColor::Empty);
        self.last_line_width.set(0.0);
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
        self.set_font_cached(font_css);
        self.set_fill_cached(color);
        self.ctx.set_text_align(match align {
            TextAlign::Start => "start",
            TextAlign::Center => "center",
            TextAlign::End => "end",
        });
        self.ctx.set_text_baseline(match baseline {
            TextBaseline::Top => "top",
            TextBaseline::Middle => "middle",
            TextBaseline::Bottom => "bottom",
            TextBaseline::Alphabetic => "alphabetic",
        });
        let _ = self.ctx.fill_text(text, x, y);
    }

    fn invalidate_cache(&self) {
        // Public escape hatch for the renderer between frames.
        // (Layers call this in their `paint()` method today.)
        self.last_fill.set(CachedColor::Empty);
        self.last_stroke.set(CachedColor::Empty);
        self.last_font.set(CachedColor::Empty);
        self.last_line_width.set(0.0);
    }

    fn apply_dpr_transform(&self, dpr: i32) {
        let dpr_f = f64::from(dpr);
        self.ctx
            .set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .expect("set_transform should not fail");
        self.ctx.scale(dpr_f, dpr_f).expect("scale should not fail");
    }

    fn reset_text_defaults(&self) {
        self.ctx.set_text_align("center");
        self.ctx.set_text_baseline("middle");
    }

    fn begin_group(&self, _class: &'static str) {}
    fn end_group(&self) {}
}
