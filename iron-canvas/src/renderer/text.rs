//! Cell text painting.
//!
//! Layout (font metrics, line wrap, per-line positioning) is resolved
//! upstream in `crate::types::text_paint`. This file just paints a
//! pre-computed `TextPaint` onto the canvas: fill each line, then
//! optionally stroke an underline / strike.

use super::RendererCore;

use crate::painter::{PaintColor, Painter, TextAlign, TextBaseline};
use crate::{
    geometry::constants::STANDARD_BORDER_WIDTH,
    renderer::text_paint::{TextColor, TextLine, TextPaint},
};

/// With `textBaseline: "middle"`, `center_y` is the em-square midpoint. The
/// typographic baseline sits at ~`center_y + font_size * 0.15`; `0.35` puts
/// the underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: i32 = 2;

impl<P: Painter> RendererCore<P> {
    /// Paint a pre-computed `TextPaint` onto the canvas. Pure pixel pusher:
    /// no model access, no layout work — everything is already resolved.
    /// `lines` is the externally owned line buffer that `TextPaint::resolve_into`
    /// just filled; passing it alongside `t` keeps the per-cell allocation off
    /// the path while preserving the old "set state then clip then stroke"
    /// ordering.
    pub(super) fn paint_text(&self, t: &TextPaint, lines: &[TextLine]) {
        // TextColor::Static carries the theme color; the helper routes
        // Cow::Borrowed through the painter's ptr-eq fast path and Cow::Owned
        // through content-eq. TextColor::Owned is a per-cell custom color
        // from the font's explicit color attribute.
        let color = match &t.color {
            TextColor::Static(s) => PaintColor::from_theme_str(s),
            TextColor::Owned(s) => PaintColor::Borrowed(s),
        };
        // `font_css` is interned `Rc<str>` per (size, weight, slant, family);
        // not `&'static`, so it goes through Borrowed (content-eq cache hit
        // across cells with the same interned font).
        let font_css = PaintColor::Borrowed(&t.font_css);
        let underline_offset =
            (t.font_size_px * UNDERLINE_OFFSET_FACTOR).max(f64::from(MIN_UNDERLINE_OFFSET));
        let stroke_w = f64::from(STANDARD_BORDER_WIDTH);

        if t.needs_clip {
            self.painter.push_clip(t.clip);
        }
        for line in lines {
            self.painter.fill_text(
                &line.text,
                line.center_x,
                line.center_y,
                font_css,
                color,
                TextAlign::Center,
                TextBaseline::Middle,
            );
            let x1 = line.center_x - line.width / 2.0;
            let x2 = line.center_x + line.width / 2.0;
            if t.underline {
                self.painter
                    .stroke_text_hline(x1, x2, line.center_y + underline_offset, color, stroke_w);
            }
            if t.strike {
                self.painter
                    .stroke_text_hline(x1, x2, line.center_y, color, stroke_w);
            }
        }
        if t.needs_clip {
            self.painter.pop_clip();
        }
    }
}
