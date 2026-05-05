//! Cell text painting.
//!
//! Layout (font metrics, line wrap, per-line positioning) is resolved
//! upstream in `crate::types::text_paint`. This file just paints a
//! pre-computed `TextPaint` onto the canvas: fill each line, then
//! optionally stroke an underline / strike.

use super::RendererCore;

use crate::{
    geometry::constants::STANDARD_BORDER_WIDTH,
    renderer::text_paint::{TextColor, TextPaint},
};

/// With `textBaseline: "middle"`, `center_y` is the em-square midpoint. The
/// typographic baseline sits at ~`center_y + font_size * 0.15`; `0.35` puts
/// the underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: i32 = 2;

impl RendererCore {
    /// Paint a pre-computed `TextPaint` onto the canvas. Pure pixel pusher:
    /// no model access, no layout work - everything is already resolved.
    pub(super) fn paint_text(&self, t: &TextPaint) {
        // Set state before `with_clip` so save/restore preserves it — the values
        // survive the restore and the cache stays valid across consecutive cells
        // that share the same font or color.
        self.set_font_cached(&t.font_css);
        // Static dispatch mirrors paint_bg in cells.rs: theme-default text
        // colors hit the pointer-eq cache fast path; per-cell overrides take
        // the value-compare path.
        match &t.color {
            TextColor::Static(s) => self.set_fill_static(s),
            TextColor::Owned(s) => self.set_fill_cached(s),
        }
        if t.underline || t.strike {
            match &t.color {
                TextColor::Static(s) => self.set_stroke_static(s),
                TextColor::Owned(s) => self.set_stroke_cached(s),
            }
            self.set_line_width_cached(STANDARD_BORDER_WIDTH);
        }

        self.with_clip(t.clip, |this| {
            let underline_offset =
                (t.font_size_px * UNDERLINE_OFFSET_FACTOR).max(f64::from(MIN_UNDERLINE_OFFSET));

            for line in &t.lines {
                this.ctx_ref()
                    .fill_text(&line.text, line.center_x, line.center_y)
                    .ok();
                let x1 = line.center_x - line.width / 2.0;
                let x2 = line.center_x + line.width / 2.0;
                if t.underline {
                    this.stroke_text_hline(x1, x2, line.center_y + underline_offset);
                }
                if t.strike {
                    this.stroke_text_hline(x1, x2, line.center_y);
                }
            }
        });
    }
}
