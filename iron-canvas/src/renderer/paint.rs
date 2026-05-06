//! Rectangle paint primitives on top of Canvas 2D.
//!
//! Every overlay, cell fill, header strip, and text clip is a rectangle; the
//! raw `ctx.fill_rect` / `stroke_rect` / `set_line_dash` calls are wrapped
//! here so callers read "what is being painted" instead of the ceremony of
//! how. `rect_*` / `stroke_*` / `with_*` prefixes distinguish these from
//! raw `ctx.*` methods at read time.

use crate::geometry::{
    constants::STANDARD_BORDER_WIDTH,
    pixel_rect::PixelRect,
    prim::{Line, Span},
};

use super::{CachedColor, RendererCore};

impl RendererCore {
    /// Fill `rect` with a solid color.
    pub(super) fn rect_fill(&self, rect: PixelRect, color: &str) {
        self.set_fill_cached(color);
        self.ctx.fill_rect(
            f64::from(rect.top_left.x),
            f64::from(rect.top_left.y),
            f64::from(rect.width),
            f64::from(rect.height),
        );
    }

    /// Stroke `rect`'s outline at `width` pixels. Width is restored to
    /// `STANDARD_BORDER_WIDTH` on exit via `with_stroke_width`.
    pub(super) fn rect_stroke(&self, rect: PixelRect, color: &str, width: i32) {
        self.set_stroke_cached(color);
        self.with_stroke_width(width, |this| {
            this.ctx.stroke_rect(
                f64::from(rect.top_left.x),
                f64::from(rect.top_left.y),
                f64::from(rect.width),
                f64::from(rect.height),
            );
        });
    }

    /// Dashed outline (4-on / 3-off). Resets dash pattern and line_width on exit.
    pub(super) fn rect_dashed(&self, rect: PixelRect, color: &str, width: i32) {
        self.ctx.set_line_dash(&self.dash_pattern).ok();
        self.rect_stroke(rect, color, width);
        self.ctx.set_line_dash(&self.dash_empty).ok();
    }

    /// Run `f` with `width` as the active stroke `line_width`. Restores
    /// `STANDARD_BORDER_WIDTH` on exit — makes the reset invariant explicit
    /// and shared by every helper that would otherwise duplicate it.
    pub(super) fn with_stroke_width<R>(&self, width: i32, f: impl FnOnce(&Self) -> R) -> R {
        self.set_line_width_cached(width);
        let result = f(self);
        self.set_line_width_cached(STANDARD_BORDER_WIDTH);
        result
    }

    /// Stroke an axis-aligned `Line`. Dispatches on the enum variant so the
    /// caller doesn't pick `stroke_hline` vs `stroke_vline` manually.
    pub(super) fn stroke_line(&self, line: Line) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, f64::from(y)),
            Line::V { x, span } => self.stroke_vline(f64::from(x), span),
        }
    }

    /// Run `f` with `rect` as the active clip region. Save/restore bracketed.
    pub(super) fn with_clip<R>(&self, rect: PixelRect, f: impl FnOnce(&Self) -> R) -> R {
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx.rect(
            f64::from(rect.top_left.x),
            f64::from(rect.top_left.y),
            f64::from(rect.width),
            f64::from(rect.height),
        );
        self.ctx.clip();
        let result = f(self);
        self.ctx.restore();
        result
    }

    /// Horizontal line from (x1, y) to (x2, y).
    pub(super) fn stroke_hline(&self, span: Span, y: f64) {
        let y = self.snap_stroke(y);
        self.ctx.begin_path();
        self.ctx.move_to(f64::from(span.from), y);
        self.ctx.line_to(f64::from(span.to), y);
        self.ctx.stroke();
    }

    /// Horizontal line from (x1, y) to (x2, y).
    pub(super) fn stroke_text_hline(&self, x1: f64, x2: f64, y: f64) {
        let y = self.snap_stroke(y);
        self.ctx.begin_path();
        self.ctx.move_to(x1, y);
        self.ctx.line_to(x2, y);
        self.ctx.stroke();
    }

    /// Vertical line from (x, y1) to (x, y2).
    pub(super) fn stroke_vline(&self, x: f64, span: Span) {
        let x = self.snap_stroke(x);
        self.ctx.begin_path();
        self.ctx.move_to(x, f64::from(span.from));
        self.ctx.line_to(x, f64::from(span.to));
        self.ctx.stroke();
    }

    /// Set fill style, skipping the JS call when unchanged. The cache hit
    /// path takes the previous value out and compares without allocating;
    /// the miss path allocates a fresh `String` because the input is dynamic.
    /// Static (theme) callsites should prefer `set_fill_static`.
    pub(super) fn set_fill_cached(&self, color: &str) {
        let prev = self.frame_cache.last_fill.take();
        if prev.matches(color) {
            self.frame_cache.last_fill.set(prev);
        } else {
            self.ctx.set_fill_style_str(color);
            self.frame_cache
                .last_fill
                .set(CachedColor::Owned(color.to_string()));
        }
    }

    /// Theme-driven fill: pointer-equality compare on hit, no allocation on
    /// miss. Use whenever the color is a `&'static str` (theme constant,
    /// precomputed tint).
    pub(super) fn set_fill_static(&self, color: &'static str) {
        let prev = self.frame_cache.last_fill.take();
        if prev.matches_static(color) {
            self.frame_cache.last_fill.set(prev);
        } else {
            self.ctx.set_fill_style_str(color);
            self.frame_cache.last_fill.set(CachedColor::Static(color));
        }
    }

    /// Set stroke style, skipping the JS call when unchanged.
    pub(super) fn set_stroke_cached(&self, color: &str) {
        let prev = self.frame_cache.last_stroke.take();
        if prev.matches(color) {
            self.frame_cache.last_stroke.set(prev);
        } else {
            self.ctx.set_stroke_style_str(color);
            self.frame_cache
                .last_stroke
                .set(CachedColor::Owned(color.to_string()));
        }
    }

    /// Theme-driven stroke variant — see `set_fill_static`.
    pub(super) fn set_stroke_static(&self, color: &'static str) {
        let prev = self.frame_cache.last_stroke.take();
        if prev.matches_static(color) {
            self.frame_cache.last_stroke.set(prev);
        } else {
            self.ctx.set_stroke_style_str(color);
            self.frame_cache.last_stroke.set(CachedColor::Static(color));
        }
    }

    /// Set font, skipping the JS call when unchanged.
    pub(super) fn set_font_cached(&self, font: &str) {
        let prev = self.frame_cache.last_font.take();
        if prev.matches(font) {
            self.frame_cache.last_font.set(prev);
        } else {
            self.ctx.set_font(font);
            self.frame_cache
                .last_font
                .set(CachedColor::Owned(font.to_string()));
        }
    }

    /// Static-font variant — pointer-equality compare on hit, no allocation on
    /// miss. Use for constant font strings (header font, etc.).
    pub(super) fn set_font_static(&self, font: &'static str) {
        let prev = self.frame_cache.last_font.take();
        if prev.matches_static(font) {
            self.frame_cache.last_font.set(prev);
        } else {
            self.ctx.set_font(font);
            self.frame_cache.last_font.set(CachedColor::Static(font));
        }
    }

    /// Set line width, skipping the JS call when unchanged.
    pub(super) fn set_line_width_cached(&self, width: i32) {
        if self.frame_cache.last_line_width.get() != f64::from(width) {
            self.ctx.set_line_width(f64::from(width));
            self.frame_cache.last_line_width.set(f64::from(width));
        }
    }
}
