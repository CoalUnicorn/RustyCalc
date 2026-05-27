//! Cell text — resolution + paint.
//!
//! Layout (font metrics, line wrap, per-line positioning) is resolved by
//! `TextPaint::resolve_into`; the resulting `TextPaint` is the renderer-ready
//! snapshot consumed by `RendererCore::paint_text`. Per the `*Paint`
//! convention, every allocation that depends on cell content lives in the
//! resolve step, not in the paint loop.
//!
//! `paint_text` is a pure pixel pusher: fill each line, then optionally
//! stroke an underline / strike. Clipping is gated on `needs_clip` so the
//! 99 % of cells that fit don't pay the `save/restore` round-trip (which
//! wipes the painter's font/fill state cache and forces the next cell's
//! binds to miss).

use std::borrow::Cow;
use std::rc::Rc;

use ironcalc_base::types::{CellType, HorizontalAlignment, Style, VerticalAlignment};

use crate::geometry::constants::STANDARD_BORDER_WIDTH;
use crate::geometry::pixel_rect::PixelRect;
use crate::painter::{PaintColor, Painter, TextAlign, TextBaseline, TextMetrics};
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::theme::CanvasTheme;

//  layout constants

/// Below this in either pixel dimension, no text is laid out at all.
const MIN_TEXT_DIM_PX: f64 = 10.0;
const CHAR_WIDTH_FACTOR: f64 = 1.0;
const LINE_HEIGHT_FACTOR: f64 = 2.0;
const TEXT_V_INSET_PX: f64 = 4.0;
const CELL_PADDING: f64 = 4.0;

//  paint constants

/// With `textBaseline: "middle"`, `center_y` is the em-square midpoint. The
/// typographic baseline sits at ~`center_y + font_size * 0.15`; `0.35` puts
/// the underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: i32 = 2;

//  types

/// Pre-resolved text paint for one cell. Pure pixel inputs — no model access
/// during paint. The `Vec<TextLine>` lives on the caller's reusable buffer
/// (parked on `FrameCache::text_lines`) so resolve never allocates per cell.
pub struct TextPaint {
    pub clip: PixelRect,
    /// Interned `ctx.font` string. `Rc::clone` on cache hit; one alloc per
    /// unique (size, bold, italic, family) tuple per renderer lifetime.
    pub font_css: Rc<str>,
    pub font_size_px: f64,
    pub color: TextColor,
    pub underline: bool,
    pub strike: bool,
    /// Resolved horizontal alignment — carried through to `paint_text` so
    /// backends that can't measure glyphs accurately (SVG) can use
    /// `text-anchor="start"` / `"end"` anchored on cell boundaries instead
    /// of the `CHAR_WIDTH_FACTOR`-approximated `center_x`.
    pub h_align: HorizontalAlignment,
    /// True when at least one line overflows horizontally or there are
    /// multiple lines (which can overflow vertically). Resolved here so
    /// `paint_text` can skip `push_clip`/`pop_clip` when the cell can't
    /// spill — clip = save/restore on Canvas2D, which wipes the painter's
    /// font/fill state cache and forces the next cell's binds to miss.
    pub needs_clip: bool,
}

/// Per-cell text color split between the theme-default path (`Static`) and
/// the per-cell-override path (`Owned`). `Static` carries `Cow<'static, str>` —
/// `Cow::Borrowed` for built-in themes ptr-eqs through the painter cache,
/// `Cow::Owned` for host-page themes content-eqs. `Owned` carries an interned
/// `Rc<str>` from `ColorIntern` so steady-state resolution is `Rc::clone`
/// after the first sighting. Mirrors `BorderColor` in `cell/borders.rs`.
pub enum TextColor {
    Static(Cow<'static, str>),
    Owned(Rc<str>),
}

/// One visual line of text inside a cell, positioned for center-aligned rendering.
pub struct TextLine {
    pub text: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
}

//  resolve

impl TextPaint {
    /// Build a `TextPaint` for `addr` at `rect` and fill `lines` with the
    /// resolved per-line text/width/position. Returns `None` (with `lines`
    /// left empty) for empty/too-small cells. Formatted value AND cell type
    /// are supplied by the caller — `render_pane` drains both from the
    /// prefetched `pane_values` / `pane_cell_types` buffers;
    /// `repaint_active_cell` reads the model directly for the active cell.
    /// Font / alignment / colour are resolved via `CellTextStyle`.
    ///
    /// The split between `TextPaint` (per-cell scalars) and the externally
    /// owned `lines` buffer is what makes the per-cell text path zero-alloc:
    /// the caller takes the buffer once at the top of `render_pane`, hands it
    /// to every cell, and parks it back on `FrameCache::text_lines`.
    pub fn resolve_into<P: Painter>(
        renderer: &RendererCore<P>,
        rect: PixelRect,
        theme: &CanvasTheme,
        style: &Style,
        text: String,
        cell_type: CellType,
        lines: &mut Vec<TextLine>,
    ) -> Option<TextPaint> {
        if text.is_empty() {
            return None;
        }
        if f64::from(rect.width) < MIN_TEXT_DIM_PX || f64::from(rect.height) < MIN_TEXT_DIM_PX {
            return None;
        }

        let CellTextStyle {
            text_color,
            underline,
            strike,
            h_align,
            v_align,
            wrap_text,
        } = CellTextStyle::resolve(cell_type, theme, style, &renderer.color_intern);

        // Font interning: skips `FontStyle::build` on cache hit. Same lookup
        // is shared across cells with identical (size, weight, slant, family).
        let size_px = f64::from(style.font.sz);
        let font_css = renderer.font_intern.get_or_build(
            size_px,
            style.font.b,
            style.font.i,
            &style.font.name,
            "Calibri",
        );

        let approx_char_w = size_px * CHAR_WIDTH_FACTOR;
        let line_height = size_px * LINE_HEIGHT_FACTOR;
        let usable_w = f64::from(rect.width) - 2.0 * CELL_PADDING;
        let right = rect.right();
        let bottom = rect.bottom();
        let center = rect.center();

        // Layout pass: split + wrap, measuring once. `lines` comes back with
        // text + width populated and `center_x/y` left at 0.0 for the position
        // pass below. Routed through `&dyn TextMetrics` so resolution stays
        // backend-agnostic.
        let mut wrap_buf = renderer.frame_cache.wrap_buf.borrow_mut();
        layout_into(
            renderer.painter(),
            &font_css,
            &text,
            wrap_text,
            usable_w,
            approx_char_w,
            lines,
            &mut wrap_buf,
        );
        drop(wrap_buf);

        let line_count = lines.len() as f64;
        for (i, line) in lines.iter_mut().enumerate() {
            let i_f = i as f64;
            let tw = line.width;
            // foreign #[non_exhaustive]: HorizontalAlignment is upstream (ironcalc).
            // Left / General / Justify / Distributed / Fill default to left-anchored.
            line.center_x = match h_align {
                HorizontalAlignment::Right => f64::from(right) - CELL_PADDING - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    f64::from(center.x)
                }
                _ => f64::from(rect.top_left.x) + CELL_PADDING + tw / 2.0,
            };
            // foreign #[non_exhaustive]: VerticalAlignment is upstream (ironcalc).
            // Top / Justify / Distributed default to top-anchored.
            line.center_y = match v_align {
                VerticalAlignment::Bottom => {
                    f64::from(bottom) - size_px / 2.0 - TEXT_V_INSET_PX
                        + (i_f - line_count + 1.0) * line_height
                }
                VerticalAlignment::Center => {
                    f64::from(center.y) + (i_f + (1.0 - line_count) / 2.0) * line_height
                }
                _ => {
                    f64::from(rect.top_left.y) + size_px / 2.0 + TEXT_V_INSET_PX + i_f * line_height
                }
            };
        }

        let needs_clip = lines.len() > 1 || lines.iter().any(|l| l.width > usable_w);

        Some(TextPaint {
            clip: rect,
            font_css,
            font_size_px: size_px,
            color: text_color,
            underline,
            strike,
            h_align,
            needs_clip,
        })
    }
}

/// Per-cell text styling resolved from the model's raw `Style`. Private step
/// inside `TextPaint::resolve_into`. Font css is interned separately via
/// `FontIntern` so this struct carries no per-cell `String`.
struct CellTextStyle {
    text_color: TextColor,
    underline: bool,
    strike: bool,
    h_align: HorizontalAlignment,
    v_align: VerticalAlignment,
    wrap_text: bool,
}

impl CellTextStyle {
    fn resolve(
        cell_type: CellType,
        theme: &CanvasTheme,
        style: &Style,
        intern: &ColorIntern,
    ) -> Self {
        // IronCalc collapses every error variant (#VALUE!, #DIV/0!, #REF!, #NAME?,
        // #NUM!, #N/A, #NULL!, #SPILL!, #CIRC!, plus IronCalc-only #ERROR!/#N/IMPL!)
        // into the single CellType::ErrorValue discriminator. All render in
        // the theme's error color; per-error-kind styling would need a new
        // model accessor.
        let text_color = if matches!(cell_type, CellType::ErrorValue) {
            TextColor::Static(theme.error_text_color.clone())
        } else {
            match style.font.color.as_deref() {
                None | Some("#000000") => TextColor::Static(theme.default_text_color.clone()),
                Some(c) => TextColor::Owned(intern.get(c)),
            }
        };

        let alignment = style.alignment.as_ref();
        let h_align = match alignment.map(|a| &a.horizontal) {
            Some(HorizontalAlignment::Right) => HorizontalAlignment::Right,
            Some(HorizontalAlignment::Center) | Some(HorizontalAlignment::CenterContinuous) => {
                HorizontalAlignment::Center
            }
            Some(HorizontalAlignment::Left) | Some(HorizontalAlignment::Fill) => {
                HorizontalAlignment::Left
            }
            // Canvas 2D has no justify/distributed - fall back to left.
            Some(HorizontalAlignment::Justify) | Some(HorizontalAlignment::Distributed) => {
                HorizontalAlignment::Left
            }
            // General or unset: numbers right, everything else left.
            // foreign #[non_exhaustive]: CellType is upstream (ironcalc) —
            // Text / LogicalValue / ErrorValue / Array / CompoundData default to left.
            None | Some(HorizontalAlignment::General) => match cell_type {
                CellType::Number => HorizontalAlignment::Right,
                _ => HorizontalAlignment::Left,
            },
        };
        let v_align = alignment
            .map(|a| a.vertical.clone())
            .unwrap_or(VerticalAlignment::Bottom);
        let wrap_text = alignment.is_some_and(|a| a.wrap_text);

        Self {
            text_color,
            underline: style.font.u,
            strike: style.font.strike,
            h_align,
            v_align,
            wrap_text,
        }
    }
}

// layout

/// Break `text` into render-ready lines: split on `\n` always, then word-wrap
/// within each split when `wrap` is on and the cell has width. `approx_char_w`
/// is the fallback glyph width when `measure_text` fails; biases the wrap
/// point slightly but never loses characters.
///
/// Fills `out` with `TextLine`s carrying `text` + `width`; `center_x`/`center_y`
/// are left at `0.0` for the caller's positioning pass.
///
/// **Measurement strategy.** Non-wrap path measures each `\n`-split line once.
/// Wrap path measures each word once plus one space-width per call, then sums
/// additively (`current_w + space_w + word_w`). Adjacent-glyph kerning across
/// a space is sub-pixel for the proportional fonts this renderer ships, well
/// below the rounding the position pass already does — additive sum is within
/// 1px of a fresh `measureText(full_line)` and avoids the O(words²) re-measure
/// the previous algorithm did on long wrapping cells.
#[allow(clippy::too_many_arguments)]
pub fn layout_into(
    metrics: &dyn TextMetrics,
    font_css: &str,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
    out: &mut Vec<TextLine>,
    wrap_buf: &mut String,
) {
    // Slot-reuse contract: do not `out.clear()` — that would drop every
    // `TextLine.text` String and free its capacity, defeating the cache.
    // Overwrite slots in place via `write_line`, then `truncate(idx)` so the
    // tail of stale lines from a previous (longer) cell isn't iterated.
    let mut idx = 0usize;
    // Backends without sticky font state (Recorder, future SVG) need the font
    // passed per-measurement; CanvasPainter caches the value internally.
    let measure = |s: &str| -> f64 {
        let w = metrics.measure_text_width(s, font_css);
        if w > 0.0 {
            w
        } else {
            s.len() as f64 * approx_char_w
        }
    };

    if !wrap || usable_w <= 0.0 {
        for line in text.split('\n') {
            let width = measure(line);
            write_line(out, &mut idx, line, width);
        }
        out.truncate(idx);
        return;
    }

    let space_w = measure(" ");

    for raw_line in text.split('\n') {
        wrap_buf.clear();
        let mut current_w = 0.0;
        for word in raw_line.split_whitespace() {
            let word_w = measure(word);
            let separator_w = if wrap_buf.is_empty() { 0.0 } else { space_w };
            let tentative_w = current_w + separator_w + word_w;

            if tentative_w > usable_w && !wrap_buf.is_empty() {
                // Overflow with prior content: commit the line as-is, start
                // fresh with this word alone (no leading space → no separator).
                write_line(out, &mut idx, wrap_buf, current_w);
                wrap_buf.clear();
                wrap_buf.push_str(word);
                current_w = word_w;
            } else {
                if !wrap_buf.is_empty() {
                    wrap_buf.push(' ');
                }
                wrap_buf.push_str(word);
                current_w = tentative_w;
            }
        }
        write_line(out, &mut idx, wrap_buf, current_w);
    }
    out.truncate(idx);

    fn write_line(out: &mut Vec<TextLine>, idx: &mut usize, text: &str, width: f64) {
        if let Some(slot) = out.get_mut(*idx) {
            slot.text.clear();
            slot.text.push_str(text);
            slot.center_x = 0.0;
            slot.center_y = 0.0;
            slot.width = width;
        } else {
            out.push(TextLine {
                text: text.to_owned(),
                center_x: 0.0,
                center_y: 0.0,
                width,
            });
        }
        *idx += 1;
    }
}

//  paint

impl<P: Painter> RendererCore<P> {
    /// Paint a pre-computed `TextPaint` onto the canvas. Pure pixel pusher:
    /// no model access, no layout work — everything is already resolved.
    /// `lines` is the externally owned line buffer that `TextPaint::resolve_into`
    /// just filled; passing it alongside `t` keeps the per-cell allocation off
    /// the path while preserving the old "set state then clip then stroke"
    /// ordering.
    pub fn paint_text(&self, t: &TextPaint, lines: &[TextLine]) {
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
            let (x, align) = match t.h_align {
                HorizontalAlignment::Right => {
                    (f64::from(t.clip.right()) - CELL_PADDING, TextAlign::End)
                }
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    (line.center_x, TextAlign::Center)
                }
                _ => {
                    // Left / General / Justify / Distributed / Fill — start-anchored
                    // on the cell's left edge. No width approximation needed; the
                    // SVG `text-anchor="start"` renders glyphs at their natural
                    // width and overflow into adjacent empty cells just works.
                    (
                        f64::from(t.clip.top_left.x) + CELL_PADDING,
                        TextAlign::Start,
                    )
                }
            };
            self.painter.fill_text(
                &line.text,
                x,
                line.center_y,
                font_css,
                color,
                align,
                TextBaseline::Middle,
            );
            let x1 = line.center_x - line.width / 2.0;
            let x2 = line.center_x + line.width / 2.0;
            if t.underline {
                self.painter.stroke_text_hline(
                    x1,
                    x2,
                    line.center_y + underline_offset,
                    color,
                    stroke_w,
                );
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
