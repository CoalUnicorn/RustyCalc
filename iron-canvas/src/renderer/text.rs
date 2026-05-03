//! Cell text painting.
//!
//! Layout (font metrics, line wrap, per-line positioning) is resolved
//! upstream in `crate::types::text_paint`. This file just paints a
//! pre-computed `TextPaint` onto the canvas: fill each line, then
//! optionally stroke an underline / strike.

use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

use crate::geometry::prim::Span;
pub(crate) use crate::types::text_paint::TextPaint;

/// With `textBaseline: "middle"`, `center_y` is the em-square midpoint. The
/// typographic baseline sits at ~`center_y + font_size * 0.15`; `0.35` puts
/// the underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: f64 = 2.0;

impl CanvasRenderer {
    /// Paint a pre-computed `TextPaint` onto the canvas. Pure pixel pusher:
    /// no model access, no layout work - everything is already resolved.
    pub(super) fn paint_text(&self, t: &TextPaint) {
        // Set state before `with_clip` so save/restore preserves it — the values
        // survive the restore and the cache stays valid across consecutive cells
        // that share the same font or color.
        self.set_font_cached(&t.font_css);
        self.set_fill_cached(t.color.as_str());
        if t.underline || t.strike {
            self.set_stroke_cached(t.color.as_str());
            self.set_line_width_cached(STANDARD_BORDER_WIDTH);
        }

        self.with_clip(t.clip, |this| {
            let underline_offset =
                (t.font_size_px * UNDERLINE_OFFSET_FACTOR).max(MIN_UNDERLINE_OFFSET);

            for line in &t.lines {
                this.ctx_ref()
                    .fill_text(
                        &line.text,
                        this.snap_pixel(line.center_x),
                        this.snap_pixel(line.center_y),
                    )
                    .ok();
                let x1 = line.center_x - line.width / 2.0;
                let x2 = line.center_x + line.width / 2.0;
                if t.underline {
                    this.stroke_hline(Span { from: x1, to: x2 }, line.center_y + underline_offset);
                }
                if t.strike {
                    this.stroke_hline(Span { from: x1, to: x2 }, line.center_y);
                }
            }
        });
    }
}
