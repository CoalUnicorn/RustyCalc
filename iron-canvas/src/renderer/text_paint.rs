//! Text-paint resolution.
//!
//! Owns layout (font metrics, line wrap, per-line positioning). The output
//! `TextPaint` is the renderer-ready snapshot consumed by
//! `RendererCore::paint_text`. Per the `*Paint` convention, every
//! allocation that depends on cell content lives here, not at paint time.

use std::borrow::Cow;
use std::rc::Rc;

use ironcalc_base::types::{CellType, HorizontalAlignment, Style, VerticalAlignment};

use crate::geometry::pixel_rect::PixelRect;
use crate::painter::{Painter, TextMetrics};
use crate::renderer::cache::ColorIntern;
use crate::renderer::RendererCore;
use crate::theme::CanvasTheme;
use crate::types::coord::CellAddress;
use crate::CanvasModel;

/// Below this in either pixel dimension, no text is laid out at all.
pub(crate) const MIN_TEXT_DIM_PX: f64 = 10.0;
const CHAR_WIDTH_FACTOR: f64 = 1.0;
const LINE_HEIGHT_FACTOR: f64 = 2.0;
const TEXT_V_INSET_PX: f64 = 4.0;
const CELL_PADDING: f64 = 4.0;

/// Pre-resolved text paint for one cell. Pure pixel inputs — no model access
/// during paint. The `Vec<TextLine>` lives on the caller's reusable buffer
/// (parked on `FrameCache::text_lines`) so resolve never allocates per cell.
pub(crate) struct TextPaint {
    pub clip: PixelRect,
    /// Interned `ctx.font` string. `Rc::clone` on cache hit; one alloc per
    /// unique (size, bold, italic, family) tuple per renderer lifetime.
    pub font_css: Rc<str>,
    pub font_size_px: f64,
    pub color: TextColor,
    pub underline: bool,
    pub strike: bool,
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
/// after the first sighting. Mirrors `BorderColor` in `renderer/cells.rs`.
pub(crate) enum TextColor {
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

impl TextPaint {
    /// Build a `TextPaint` for `addr` at `rect` and fill `lines` with the
    /// resolved per-line text/width/position. Returns `None` (with `lines`
    /// left empty) for empty/too-small cells. The formatted value is supplied
    /// by the caller — `render_pane` pulls it out of the prefetched
    /// `pane_values` buffer; `repaint_active_cell` reads the model directly.
    /// Font / alignment / colour are resolved via `CellTextStyle`.
    ///
    /// The split between `TextPaint` (per-cell scalars) and the externally
    /// owned `lines` buffer is what makes the per-cell text path zero-alloc:
    /// the caller takes the buffer once at the top of `render_pane`, hands it
    /// to every cell, and parks it back on `FrameCache::text_lines`.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve_into<P: Painter>(
        renderer: &RendererCore<P>,
        model: &dyn CanvasModel,
        addr: CellAddress,
        rect: PixelRect,
        theme: &CanvasTheme,
        style: &Style,
        text: String,
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
        } = CellTextStyle::resolve(
            model,
            addr.sheet,
            addr.row,
            addr.column,
            theme,
            style,
            &renderer.color_intern,
        );

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
            line.center_x = match h_align {
                HorizontalAlignment::Right => f64::from(right) - CELL_PADDING - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    f64::from(center.x)
                }
                _ => f64::from(rect.top_left.x) + CELL_PADDING + tw / 2.0,
            };
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
        model: &dyn CanvasModel,
        sheet: u32,
        row: i32,
        column: i32,
        theme: &CanvasTheme,
        style: &Style,
        intern: &ColorIntern,
    ) -> Self {
        let cell_type = model
            .get_cell_type(sheet, row, column)
            .unwrap_or(CellType::Text);

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
            None | Some(HorizontalAlignment::General) => match cell_type {
                CellType::Number => HorizontalAlignment::Right,
                _ => HorizontalAlignment::Left,
            },
        };
        let v_align = alignment
            .map(|a| a.vertical.clone())
            .unwrap_or(VerticalAlignment::Bottom);
        let wrap_text = alignment.map(|a| a.wrap_text).unwrap_or(false);

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
pub(crate) fn layout_into(
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
