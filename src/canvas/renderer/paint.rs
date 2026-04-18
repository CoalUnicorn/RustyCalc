//! Rectangle paint primitives on top of Canvas 2D.
//!
//! Every overlay, cell fill, header strip, and text clip is a rectangle; the
//! raw `ctx.fill_rect` / `stroke_rect` / `set_line_dash` calls are wrapped
//! here so callers read "what is being painted" instead of the ceremony of
//! how. `paint_*` prefixes distinguish these from raw `ctx.*` methods at
//! read time.

use super::super::geometry::PixelRect;
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

impl CanvasRenderer {
    /// Fill `rect` with a solid color.
    pub(super) fn rect_fill(&self, rect: PixelRect, color: &str) {
        self.ctx.set_fill_style_str(color);
        self.ctx
            .fill_rect(rect.point.x, rect.point.y, rect.width, rect.height);
    }

    /// Stroke `rect`'s outline at `width` pixels. Restores line_width to
    /// `STANDARD_BORDER_WIDTH` on exit so subsequent strokes don't inherit.
    pub(super) fn rect_stroke(&self, rect: PixelRect, color: &str, width: f64) {
        self.ctx.set_stroke_style_str(color);
        self.ctx.set_line_width(width);
        self.ctx
            .stroke_rect(rect.point.x, rect.point.y, rect.width, rect.height);
        self.ctx.set_line_width(STANDARD_BORDER_WIDTH);
    }

    /// Dashed outline (4-on / 3-off). Resets dash pattern and line_width on exit.
    pub(super) fn rect_dashed(&self, rect: PixelRect, color: &str, width: f64) {
        let dash = web_sys::js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into());
        self.ctx.set_line_dash(&dash).ok();
        self.rect_stroke(rect, color, width);
        self.ctx.set_line_dash(&web_sys::js_sys::Array::new()).ok();
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

    /// horizontal
    pub(super) fn stroke_hline(&self, x1: f64, x2: f64, y: f64) {
        self.ctx.begin_path();
        self.ctx.move_to(x1, y);
        self.ctx.line_to(x2, y);
        self.ctx.stroke();
    }
}
