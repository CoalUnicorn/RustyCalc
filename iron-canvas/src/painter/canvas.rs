//! Canvas 2D backend for `Painter`.
//!
//! Owns the `CanvasRenderingContext2d` plus the per-frame state cache
//! (`Cell<CachedColor>` for fill/stroke/font, `Cell<f64>` for line_width).
//! The cache is a Canvas2D-only optimization — it lives here, not in the
//! trait, because Recorder/SVG backends do not need it.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use web_sys::{js_sys, CanvasRenderingContext2d};

use super::{PaintColor, Painter, Sealed, TextAlign, TextBaseline, TextMetrics};
use crate::geometry::{
    pixel_rect::PixelRect,
    prim::{Line, Span},
};
use crate::wasm::diag::console_warn;

/// One-shot guard for the `measure_text` fallback warning. Process-wide
/// (not per-painter) so grid + overlay layers share a single signal —
/// the failure mode is a ctx-level capability, not a layer-local one.
static MEASURE_WARN_EMITTED: AtomicBool = AtomicBool::new(false);

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
    /// Mirror of the active ctx.scale factor, written by every
    /// `apply_dpr_transform`. Read by `blit` so the source rect (which
    /// reads from the DPR-scaled backing store) is sized in backing-store
    /// pixels — dest coords go through the active transform unchanged.
    pub dpr: Cell<i32>,
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
            dpr: Cell::new(1),
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
        match self.ctx.measure_text(text) {
            Ok(m) => m.width(),
            Err(_) => {
                // Debug builds: crash on first occurrence so a regressing
                // ctx state (lost context, bad font_css) is caught loud
                // in dev. Release: char-count fallback survives, and a
                // single console.warn whispers once per session.
                debug_assert!(
                    false,
                    "CanvasPainter::measure_text_width: ctx.measure_text errored; \
                     falling back to char-count×6 for {text:?} font={font_css:?}"
                );
                if MEASURE_WARN_EMITTED
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    console_warn(
                        "iron-canvas: ctx.measure_text errored; using char-count fallback \
                         (subsequent measure errors silenced)",
                    );
                }
                text.chars().count() as f64 * 6.0
            }
        }
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
        self.dpr.set(dpr);
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

    fn supports_blit(&self) -> bool {
        true
    }

    fn blit(&self, src: PixelRect, dst: PixelRect) {
        // `ctx.canvas()` is None only for a detached OffscreenCanvas ctx,
        // which iron-canvas never constructs — but the API returns Option,
        // so dropping on None keeps blit a silent no-op rather than a panic.
        let Some(canvas) = self.ctx.canvas() else {
            return;
        };
        // Source coords address the DPR-scaled backing store; dest coords
        // flow through the active ctx.scale(dpr,dpr) transform, so only
        // src multiplies explicitly. This matches the asymmetry of
        // CanvasRenderingContext2D.drawImage's source/destination spaces.
        let dpr = f64::from(self.dpr.get());
        let sx = f64::from(src.top_left.x) * dpr;
        let sy = f64::from(src.top_left.y) * dpr;
        let sw = f64::from(src.width) * dpr;
        let sh = f64::from(src.height) * dpr;
        let dx = f64::from(dst.top_left.x);
        let dy = f64::from(dst.top_left.y);
        let dw = f64::from(dst.width);
        let dh = f64::from(dst.height);
        #[cfg(target_arch = "wasm32")]
        {
            use crate::wasm::diag::console_log;
            let cw = f64::from(canvas.width());
            let ch = f64::from(canvas.height());
            let oob_y = sy + sh > ch;
            let oob_x = sx + sw > cw;
            console_log(&format!(
                "[blit] backing={}x{} dpr={} src_css=({},{},{}x{}) dst_css=({},{},{}x{}) src_backing=({},{},{}x{}) OOB_x={} OOB_y={}",
                cw, ch, self.dpr.get(),
                src.top_left.x, src.top_left.y, src.width, src.height,
                dst.top_left.x, dst.top_left.y, dst.width, dst.height,
                sx, sy, sw, sh,
                oob_x, oob_y,
            ));
        }
        let _ = self
            .ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &canvas, sx, sy, sw, sh, dx, dy, dw, dh,
            );
    }
}
