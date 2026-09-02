//! Canvas 2D backend for `Painter`.
//!
//! Owns the `CanvasRenderingContext2d` plus the per-frame state cache
//! (`Cell<CachedColor>` for fill/stroke/font, `Cell<f64>` for line_width).
//! The cache is a Canvas2D-only optimization — it lives here, not in the
//! trait, because Recorder/SVG backends do not need it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, js_sys};

use iron_canvas_core::geometry::{
    constants::{DASHED_RECT_PATTERN, STANDARD_BORDER_WIDTH},
    pixel_rect::PixelRect,
    prim::{Line, Point, Span},
};
use iron_canvas_core::painter::{
    BlitPainter, GroupClass, PaintColor, Painter, TextAlign, TextBaseline, TextMetrics,
    approx_text_width, parse_font_size_px,
};

use crate::measure_cache::MeasureCache;

// Private `console.warn` binding — zero IronCalc, kept local so this crate
// does not depend on `iron-canvas-web`'s `wasm` diagnostics module.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = warn)]
    fn console_warn(s: &str);
}

/// One-shot guard for the `measure_text` fallback warning. Process-wide
/// (not per-painter) so grid + overlay layers share a single signal —
/// the failure mode is a ctx-level capability, not a layer-local one.
static MEASURE_WARN_EMITTED: AtomicBool = AtomicBool::new(false);

/// Snap an axis-aligned stroke's cross-axis coordinate onto the pixel grid.
///
/// Canvas centers a stroke on its path, so a width-1 line at integer `coord`
/// covers `coord-0.5 .. coord+0.5` and antialiases into two half-opacity
/// columns — fuzzy borders whose corners read as a faint/missing pixel where
/// two perpendicular edges meet. Odd-width strokes are crisp when centered on
/// a half-pixel; even-width strokes when centered on the integer. Mirrors the
/// `+ 0.5` trick `draw_corner_box` already uses for frozen separators.
fn snap_stroke_cross(coord: f64, width: f64) -> f64 {
    if (width.round() as i32) % 2 == 0 {
        coord.round()
    } else {
        coord.floor() + 0.5
    }
}

/// Cached color/font value. `Static` is the zero-alloc fast path: when the
/// renderer pushed a `&'static str` (theme color, `HEADER_FONT`), we keep the
/// reference and ptr-eq it on the next call. `Owned` carries a custom color
/// that originated as a non-static `&str`, deduped to a painter-lifetime
/// `Rc<str>` (see `intern_borrowed`) so a recurring color is `Rc::clone`, not
/// a fresh allocation. `Empty` is the initial / post-clip state — always
/// misses so the next paint re-binds the ctx.
#[derive(Default, Clone)]
pub(crate) enum CachedColor {
    #[default]
    Empty,
    Static(&'static str),
    Owned(Rc<str>),
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
            (CachedColor::Owned(a), other) => &**a == other.as_str(),
        }
    }
}

/// Sticky `ctx.set_*` state cache for `CanvasPainter`. Lifted out as its
/// own type so the strategy arms in `Orchestrator::render_pending` own the
/// invalidation contract explicitly: `render_full_rebuild` and
/// `render_changed_cells` invalidate at the arm prologue (ctx state
/// about to change); `render_scroll_blit` and
/// `render_overlay_only` preserve it (blit and overlay-only do not touch grid
/// ctx state). Other painter backends (SvgPainter, RecorderPainter) have
/// no analog — the cache is a Canvas2D-specific optimization.
#[derive(Default)]
pub(crate) struct SetterCache {
    pub(crate) last_fill: Cell<CachedColor>,
    pub(crate) last_stroke: Cell<CachedColor>,
    pub(crate) last_font: Cell<CachedColor>,
    pub(crate) last_line_width: Cell<f64>,
}

impl SetterCache {
    /// Clear all four sticky binds. Called from the trait-level
    /// `invalidate_cache`, from `pop_clip` (ctx.restore wipes ctx state),
    /// and — via `RendererCore::invalidate_paint_cache` — from the paint
    /// strategy arms that change ctx fillStyle/strokeStyle/font/lineWidth.
    pub(crate) fn invalidate(&self) {
        self.last_fill.set(CachedColor::Empty);
        self.last_stroke.set(CachedColor::Empty);
        self.last_font.set(CachedColor::Empty);
        self.last_line_width.set(0.0);
    }
}

pub struct CanvasPainter {
    pub ctx: CanvasRenderingContext2d,
    /// Cross-canvas blit source. `Some` on the double-buffered grid layer:
    /// `blit` reads the kept band from the front (visible) canvas instead of
    /// self-copying `ctx`'s own canvas — same-canvas `drawImage` is the
    /// interpolation hazard documented in `apply_dpr_transform`.
    blit_src: Option<HtmlCanvasElement>,
    pub(crate) setter_cache: SetterCache,
    pub dash_pattern: js_sys::Array,
    pub dash_empty: js_sys::Array,
    pub clip_depth: Cell<u32>,
    /// Mirror of the active ctx.scale factor, written by every
    /// `apply_dpr_transform`. Read by `blit` so the source rect (which
    /// reads from the DPR-scaled backing store) is sized in backing-store
    /// pixels — dest coords go through the active transform unchanged.
    pub dpr: Cell<f64>,
    /// Painter-lifetime dedup of custom (`Borrowed`) color strings to
    /// `Rc<str>`. Distinct from `SetterCache`: `invalidate` resets the sticky
    /// binds, but the palette outlives invalidation — an interned color is
    /// still a valid key. Not cleared, so cardinality tracks the sheet's
    /// distinct-color set (bounded, like `ColorIntern`).
    palette: RefCell<Vec<Rc<str>>>,
    /// Memo of `ctx.measure_text` widths keyed `(font_css, text)`.
    /// Interior mutability because `TextMetrics::measure_text_width` takes
    /// `&self`. `get` and `insert` are separate short borrows — never held
    /// across the JS measure call. Content-keyed, so `SetterCache`
    /// invalidation must NOT touch it; only `clear_measure_cache` (font
    /// load) empties it.
    measure_cache: RefCell<MeasureCache>,
}

impl CanvasPainter {
    pub fn new(ctx: CanvasRenderingContext2d) -> Self {
        Self {
            ctx,
            blit_src: None,
            setter_cache: SetterCache::default(),
            dash_pattern: js_sys::Array::of2(
                &DASHED_RECT_PATTERN[0].into(),
                &DASHED_RECT_PATTERN[1].into(),
            ),
            dash_empty: js_sys::Array::new(),
            clip_depth: Cell::new(0),
            dpr: Cell::new(1.0),
            palette: RefCell::new(Vec::new()),
            measure_cache: RefCell::new(MeasureCache::default()),
        }
    }

    /// Construct a painter whose `blit` reads the kept band from `src` (the
    /// front/visible canvas) instead of `ctx`'s own backing store. Used for
    /// the double-buffered grid layer; see `blit_src`.
    pub fn with_blit_source(ctx: CanvasRenderingContext2d, src: HtmlCanvasElement) -> Self {
        let mut painter = Self::new(ctx);
        painter.blit_src = Some(src);
        painter
    }

    pub(crate) fn set_fill_cached(&self, color: PaintColor) {
        let prev = self.setter_cache.last_fill.take();
        if prev.matches(color) {
            self.setter_cache.last_fill.set(prev);
            return;
        }
        self.ctx.set_fill_style_str(color.as_str());
        self.setter_cache.last_fill.set(self.cache_color(color));
    }

    fn set_stroke_cached(&self, color: PaintColor) {
        let prev = self.setter_cache.last_stroke.take();
        if prev.matches(color) {
            self.setter_cache.last_stroke.set(prev);
            return;
        }
        self.ctx.set_stroke_style_str(color.as_str());
        self.setter_cache.last_stroke.set(self.cache_color(color));
    }

    pub(crate) fn set_font_cached(&self, font: PaintColor) {
        let prev = self.setter_cache.last_font.take();
        if prev.matches(font) {
            self.setter_cache.last_font.set(prev);
            return;
        }
        self.ctx.set_font(font.as_str());
        self.setter_cache.last_font.set(self.cache_color(font));
    }

    pub(crate) fn set_line_width_cached(&self, width: f64) {
        if (self.setter_cache.last_line_width.get() - width).abs() > f64::EPSILON {
            self.ctx.set_line_width(width);
            self.setter_cache.last_line_width.set(width);
        }
    }

    pub(crate) fn with_stroke_width<F: FnOnce(&Self)>(&self, width: f64, f: F) {
        self.set_line_width_cached(width);
        f(self);
        self.set_line_width_cached(f64::from(STANDARD_BORDER_WIDTH));
    }

    /// Map a call-site `PaintColor` to its `CachedColor`. `Static` stays
    /// zero-alloc; `Borrowed` is deduped through the painter palette so a
    /// recurring color reuses its `Rc<str>` instead of reallocating.
    fn cache_color(&self, color: PaintColor<'_>) -> CachedColor {
        match color {
            PaintColor::Static(s) => CachedColor::Static(s),
            PaintColor::Borrowed(s) => CachedColor::Owned(self.intern_borrowed(s)),
        }
    }

    /// Dedup a custom (`Borrowed`) color string to a painter-lifetime
    /// `Rc<str>`, so a color seen before is an `Rc::clone` rather than a fresh
    /// allocation. Cardinality is bounded by the sheet's palette (same
    /// assumption as `ColorIntern`).
    fn intern_borrowed(&self, s: &str) -> Rc<str> {
        let mut palette = self.palette.borrow_mut();
        if let Some(rc) = palette.iter().find(|rc| &***rc == s) {
            return Rc::clone(rc);
        }
        let rc: Rc<str> = Rc::from(s);
        palette.push(Rc::clone(&rc));
        rc
    }

    /// Drop all memoized text widths. Called via the facades'
    /// `fontsChanged()` when `document.fonts` finishes loading — widths
    /// measured against a fallback font are stale the moment the real
    /// font arrives.
    pub fn clear_measure_cache(&self) {
        self.measure_cache.borrow_mut().clear();
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

impl TextMetrics for CanvasPainter {
    fn measure_text_width(&self, text: &str, font_css: &str) -> f64 {
        if let Some(w) = self.measure_cache.borrow().get(font_css, text) {
            return w;
        }
        // Metrics callers (FontIntern + HEADER_FONT only feed `fill_text`)
        // pass an `&str` here. Treat as Borrowed — the cache will still hit
        // against a previously cached Static via content-eq if it was the
        // same literal.
        self.set_font_cached(PaintColor::Borrowed(font_css));
        match self.ctx.measure_text(text) {
            Ok(m) => {
                let w = m.width();
                self.measure_cache.borrow_mut().insert(font_css, text, w);
                w
            }
            Err(_) => {
                // Debug builds: crash on first occurrence so a regressing
                // ctx state (lost context, bad font_css) is caught loud
                // in dev. Release: fall back to the shared `approx_text_width`
                // estimate so a measure error agrees with the SVG/PDF/recorder
                // backends instead of diverging; a single console.warn whispers
                // once per session. Deliberately NOT cached: a broken ctx must
                // keep remeasuring so recovery heals.
                debug_assert!(
                    false,
                    "CanvasPainter::measure_text_width: ctx.measure_text errored; \
                     falling back to approx_text_width for {text:?} font={font_css:?}"
                );
                if MEASURE_WARN_EMITTED
                    .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
                {
                    console_warn(
                        "iron-canvas: ctx.measure_text errored; using approx_text_width fallback \
                         (subsequent measure errors silenced)",
                    );
                }
                approx_text_width(parse_font_size_px(font_css), text)
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

    fn fill_path(&self, points: &[Point], color: PaintColor) {
        if points.len() < 2 {
            return; // empty or single-point is a no-op
        }
        self.set_fill_cached(color);
        self.ctx.begin_path();
        let first = points[0];
        self.ctx.move_to(f64::from(first.x), f64::from(first.y));
        for p in &points[1..] {
            self.ctx.line_to(f64::from(p.x), f64::from(p.y));
        }
        self.ctx.close_path();
        self.ctx.fill();
    }

    fn clear_rect(&self, rect: PixelRect) {
        let (x, y, w, h) = rect.as_f64_tuple();
        self.ctx.clear_rect(x, y, w, h);
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
        let y = snap_stroke_cross(y, width);
        self.set_stroke_cached(color);
        self.set_line_width_cached(width);
        self.ctx.begin_path();
        self.ctx.move_to(f64::from(span.from), y);
        self.ctx.line_to(f64::from(span.to), y);
        self.ctx.stroke();
    }

    fn stroke_vline(&self, x: f64, span: Span, color: PaintColor, width: f64) {
        let x = snap_stroke_cross(x, width);
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
        self.setter_cache.invalidate();
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
        self.setter_cache.invalidate();
    }

    fn apply_dpr_transform(&self, dpr: f64) {
        self.dpr.set(dpr);
        let dpr_f = dpr;
        self.ctx
            .set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
            .expect("set_transform should not fail");
        self.ctx.scale(dpr_f, dpr_f).expect("scale should not fail");
        // Canvas2D defaults `imageSmoothingEnabled = true`, which bilinear-
        // interpolates `drawImage` source pixels. `Painter::blit` without a
        // `blit_src` calls drawImage with src/dst on the same canvas — every
        // interpolation pass smudges 1-pixel edges in the kept band. After a
        // scroll round-trip those smudges visibly accumulate as horizontal /
        // vertical lines across cells that should be empty. The grid layer's
        // cross-canvas `blit_src` (see the `blit_src` field doc) avoids the
        // self-copy entirely, but this stays disabled for both paths because
        // the ctx state is wiped on every canvas resize and this method is
        // the canonical re-init point.
        self.ctx.set_image_smoothing_enabled(false);
    }

    fn reset_text_defaults(&self) {
        self.ctx.set_text_align("center");
        self.ctx.set_text_baseline("middle");
    }

    fn begin_group(&self, _class: GroupClass) {}
    fn end_group(&self) {}
}

impl BlitPainter for CanvasPainter {
    fn blit(&self, src: PixelRect, dst: PixelRect) {
        // Prefer the cross-canvas source (front buffer). Fall back to the
        // painter's own canvas for direct-drawn surfaces; `ctx.canvas()` is
        // None only for a detached OffscreenCanvas ctx, which iron-canvas
        // never constructs — dropping on None keeps blit a silent no-op.
        let fallback;
        let canvas: &HtmlCanvasElement = if let Some(front) = &self.blit_src {
            front
        } else {
            let Some(own) = self.ctx.canvas() else {
                return;
            };
            fallback = own;
            &fallback
        };
        // Source coords address the DPR-scaled backing store; dest coords
        // flow through the active ctx.scale(dpr,dpr) transform, so only
        // src multiplies explicitly. This matches the asymmetry of
        // CanvasRenderingContext2D.drawImage's source/destination spaces.
        let dpr = self.dpr.get();
        let (sx0, sy0, sw0, sh0) = src.as_f64_tuple();
        let (dx, dy, dw, dh) = dst.as_f64_tuple();
        let (sx, sy, sw, sh) = (sx0 * dpr, sy0 * dpr, sw0 * dpr, sh0 * dpr);

        let _ = self
            .ctx
            .draw_image_with_html_canvas_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                canvas, sx, sy, sw, sh, dx, dy, dw, dh,
            );
    }
}
