//! Rectangle paint primitives on top of Canvas 2D.
//!
//! Every overlay, cell fill, header strip, and text clip is a rectangle; the
//! raw `ctx.fill_rect` / `stroke_rect` / `set_line_dash` calls are wrapped
//! here so callers read "what is being painted" instead of the ceremony of
//! how. `paint_*` prefixes distinguish these from raw `ctx.*` methods at
//! read time.

use super::super::geometry::{Line, PixelRect};
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

impl CanvasRenderer {
    /// Fill `rect` with a solid color.
    pub(super) fn rect_fill(&self, rect: PixelRect, color: &str) {
        self.ctx.set_fill_style_str(color);
        self.ctx
            .fill_rect(rect.point.x, rect.point.y, rect.width, rect.height);
    }

    /// Stroke `rect`'s outline at `width` pixels. Width is restored to
    /// `STANDARD_BORDER_WIDTH` on exit via `with_stroke_width`.
    pub(super) fn rect_stroke(&self, rect: PixelRect, color: &str, width: f64) {
        self.ctx.set_stroke_style_str(color);
        self.with_stroke_width(width, |this| {
            this.ctx
                .stroke_rect(rect.point.x, rect.point.y, rect.width, rect.height);
        });
    }

    /// Dashed outline (4-on / 3-off). Resets dash pattern and line_width on exit.
    pub(super) fn rect_dashed(&self, rect: PixelRect, color: &str, width: f64) {
        let dash = web_sys::js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into());
        self.ctx.set_line_dash(&dash).ok();
        self.rect_stroke(rect, color, width);
        self.ctx.set_line_dash(&web_sys::js_sys::Array::new()).ok();
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
            Line::H { x1, x2, y } => self.stroke_hline(x1, x2, y),
            Line::V { x, y1, y2 } => self.stroke_vline(x, y1, y2),
        }
    }

    /// Run `f` with `rect` as the active clip region. Save/restore bracketed.
    pub(super) fn with_clip<R>(&self, rect: PixelRect, f: impl FnOnce(&Self) -> R) -> R {
        self.ctx.save();
        self.ctx.begin_path();
        self.ctx
            .rect(rect.point.x, rect.point.y, rect.width, rect.height);
        self.ctx.clip();
        let result = f(self);
        self.ctx.restore();
        result
    }

    /// Horizontal line from (x1, y) to (x2, y). Path-only — caller owns
    /// `set_stroke_style_str` and `set_line_width`.
    pub(super) fn stroke_hline(&self, x1: f64, x2: f64, y: f64) {
        self.ctx.begin_path();
        self.ctx.move_to(x1, y);
        self.ctx.line_to(x2, y);
        self.ctx.stroke();
    }

    /// Vertical line from (x, y1) to (x, y2). Path-only — caller owns
    /// `set_stroke_style_str` and `set_line_width`.
    pub(super) fn stroke_vline(&self, x: f64, y1: f64, y2: f64) {
        self.ctx.begin_path();
        self.ctx.move_to(x, y1);
        self.ctx.line_to(x, y2);
        self.ctx.stroke();
    }
}
