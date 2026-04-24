//! Rectangle paint primitives on top of Canvas 2D.
//!
//! Every overlay, cell fill, header strip, and text clip is a rectangle; the
//! raw `ctx.fill_rect` / `stroke_rect` / `set_line_dash` calls are wrapped
//! here so callers read "what is being painted" instead of the ceremony of
//! how. `rect_*` / `stroke_*` / `with_*` prefixes distinguish these from
//! raw `ctx.*` methods at read time.

use crate::Span;

use super::super::geometry::{Line, PixelRect};
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

impl CanvasRenderer {
    /// Fill `rect` with a solid color.
    pub(super) fn rect_fill(&self, rect: PixelRect, color: &str) {
        self.ctx.set_fill_style_str(color);
        self.ctx
            .fill_rect(rect.top_left.x, rect.top_left.y, rect.width, rect.height);
    }

    /// Stroke `rect`'s outline at `width` pixels. Width is restored to
    /// `STANDARD_BORDER_WIDTH` on exit via `with_stroke_width`.
    pub(super) fn rect_stroke(&self, rect: PixelRect, color: &str, width: f64) {
        self.ctx.set_stroke_style_str(color);
        self.with_stroke_width(width, |this| {
            this.ctx
                .stroke_rect(rect.top_left.x, rect.top_left.y, rect.width, rect.height);
        });
    }

    /// Dashed outline (4-on / 3-off). Resets dash pattern and line_width on exit.
    pub(super) fn rect_dashed(&self, rect: PixelRect, color: &str, width: f64) {
        self.ctx.set_line_dash(&self.dash_pattern).ok();
        self.rect_stroke(rect, color, width);
        self.ctx.set_line_dash(&self.dash_empty).ok();
    }

    /// Run `f` with `width` as the active stroke `line_width`. Restores
    /// `STANDARD_BORDER_WIDTH` on exit — makes the reset invariant explicit
    /// and shared by every helper that would otherwise duplicate it.
    pub(super) fn with_stroke_width<R>(&self, width: f64, f: impl FnOnce(&Self) -> R) -> R {
        self.ctx.set_line_width(width);
        let result = f(self);
        self.ctx.set_line_width(STANDARD_BORDER_WIDTH);
        result
    }

    /// Stroke an axis-aligned `Line`. Dispatches on the enum variant so the
    /// caller doesn't pick `stroke_hline` vs `stroke_vline` manually.
    pub(super) fn stroke_line(&self, line: Line) {
        match line {
            Line::H { span, y } => self.stroke_hline(span, y),
            Line::V { x, span } => self.stroke_vline(x, span),
        }
    }

    /// Run `f` with `rect` as the active clip region. Save/restore bracketed.
    pub(super) fn with_clip<R>(&self, rect: PixelRect, f: impl FnOnce(&Self) -> R) -> R {
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx
            .rect(rect.top_left.x, rect.top_left.y, rect.width, rect.height);
        self.ctx.clip();
        let result = f(self);
        self.ctx.restore();
        result
    }

    /// Horizontal line from (x1, y) to (x2, y). Path-only — caller owns
    /// `set_stroke_style_str` and `set_line_width`.
    pub(super) fn stroke_hline(&self, span: Span, y: f64) {
        self.ctx.begin_path();
        self.ctx.move_to(span.from, y);
        self.ctx.line_to(span.to, y);
        self.ctx.stroke();
    }

    /// Vertical line from (x, y1) to (x, y2). Path-only — caller owns
    /// `set_stroke_style_str` and `set_line_width`.
    pub(super) fn stroke_vline(&self, x: f64, span: Span) {
        self.ctx.begin_path();
        self.ctx.move_to(x, span.from);
        self.ctx.line_to(x, span.to);
        self.ctx.stroke();
    }
}
