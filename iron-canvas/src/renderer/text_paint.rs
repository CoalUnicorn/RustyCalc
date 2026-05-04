//! Text-paint resolution.
//!
//! Owns layout (font metrics, line wrap, per-line positioning). The output
//! `TextPaint` is the renderer-ready snapshot consumed by
//! `CanvasRenderer::paint_text`. Per the `*Paint` convention, every
//! allocation that depends on cell content lives here, not at paint time.

use std::rc::Rc;

use ironcalc_base::types::{CellType, HorizontalAlignment, Style, VerticalAlignment};
use web_sys::CanvasRenderingContext2d;

use crate::geometry::pixel_rect::PixelRect;
use crate::renderer::CanvasRenderer;
use crate::theme::CanvasTheme;
use crate::types::coord::{CellAddress, CssColor};
use crate::CanvasModel;

/// Below this in either pixel dimension, no text is laid out at all.
pub(crate) const MIN_TEXT_DIM_PX: f64 = 10.0;
const CHAR_WIDTH_FACTOR: f64 = 1.0;
const LINE_HEIGHT_FACTOR: f64 = 2.0;
const TEXT_V_INSET_PX: f64 = 4.0;
const CELL_PADDING: f64 = 4.0;

/// Pre-resolved text paint for one cell. Pure pixel inputs - no model access
/// during paint.
pub(crate) struct TextPaint {
    pub clip: PixelRect,
    /// Interned `ctx.font` string. `Rc::clone` on cache hit; one alloc per
    /// unique (size, bold, italic, family) tuple per renderer lifetime.
    pub font_css: Rc<str>,
    pub font_size_px: f64,
    pub color: TextColor,
    pub underline: bool,
    pub strike: bool,
    pub lines: Vec<TextLine>,
}

/// Per-cell text color split between the zero-alloc theme-default fast path
/// (`Static`) and the allocating per-cell-override path (`Owned`).
/// Mirrors `BorderColor` in `renderer/cells.rs`.
pub(crate) enum TextColor {
    Static(&'static str),
    Owned(Box<str>),
}

/// One visual line of text inside a cell, positioned for center-aligned rendering.
pub struct TextLine {
    pub text: String,
    pub center_x: f64,
    pub center_y: f64,
    pub width: f64,
}

impl TextPaint {
    /// Build a `TextPaint` for `addr` at `rect`, or `None` for empty/too-small
    /// cells. Reads the formatted value from the model and resolves font /
    /// alignment / colour via `CellTextStyle`.
    pub fn resolve(
        renderer: &CanvasRenderer,
        model: &dyn CanvasModel,
        addr: CellAddress,
        rect: PixelRect,
        style: &Style,
    ) -> Option<TextPaint> {
        let text = model.get_formatted_cell_value(addr.sheet, addr.row, addr.column)?;
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
            renderer.theme(),
            style,
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

        // Set font on ctx so measure_text() returns accurate widths.
        let ctx = renderer.ctx_ref();
        ctx.set_font(&font_css);

        let text_lines = layout_lines(ctx, &text, wrap_text, usable_w, approx_char_w);

        let line_count = text_lines.len() as f64;
        let mut lines: Vec<TextLine> = Vec::new();

        for (i, line) in text_lines.into_iter().enumerate() {
            let tw = ctx
                .measure_text(&line)
                .map(|m| m.width())
                .unwrap_or(line.len() as f64 * approx_char_w);
            let i_f = i as f64;
            let center_x = match h_align {
                HorizontalAlignment::Right => f64::from(right) - CELL_PADDING - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    f64::from(center.x)
                }
                _ => f64::from(rect.top_left.x) + CELL_PADDING + tw / 2.0,
            };
            let center_y = match v_align {
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
            lines.push(TextLine {
                text: line,
                center_x,
                center_y,
                width: tw,
            });
        }

        Some(TextPaint {
            clip: rect,
            font_css,
            font_size_px: size_px,
            color: text_color,
            underline,
            strike,
            lines,
        })
    }
}

/// Per-cell text styling resolved from the model's raw `Style`. Private step
/// inside `TextPaint::resolve`; not exported. Font css is interned separately
/// via `FontIntern` so this struct carries no per-cell `String`.
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
    ) -> Self {
        let cell_type = model
            .get_cell_type(sheet, row, column)
            .unwrap_or(CellType::Text);

        // IronCalc collapses every error variant (#VALUE!, #DIV/0!, #REF!, #NAME?,
        // #NUM!, #N/A, #NULL!, #SPILL!, #CIRC!, plus IronCalc-only #ERROR!/#N/IMPL!)
        // into the single CellType::ErrorValue discriminator. Color them all the
        // same theme red — per-error-kind styling would require a new model
        // accessor (see xlsm_err.md "Renderer-side categorisation").
        let text_color = if matches!(cell_type, CellType::ErrorValue) {
            TextColor::Static(theme.error_text_color)
        } else {
            match style.font.color.as_deref() {
                None | Some("#000000") => TextColor::Static(theme.default_text_color),
                Some(c) => TextColor::Owned(CssColor::new(c).into_string().into_boxed_str()),
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
fn layout_lines(
    ctx: &CanvasRenderingContext2d,
    text: &str,
    wrap: bool,
    usable_w: f64,
    approx_char_w: f64,
) -> Vec<String> {
    if !wrap || usable_w <= 0.0 {
        return text.split('\n').map(str::to_owned).collect();
    }
    let mut result: Vec<String> = Vec::new();
    for raw_line in text.split('\n') {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_owned()
            } else {
                format!("{current} {word}")
            };
            let w = ctx
                .measure_text(&candidate)
                .map(|m| m.width())
                .unwrap_or(candidate.len() as f64 * approx_char_w);
            if w <= usable_w || current.is_empty() {
                current = candidate;
            } else {
                result.push(current);
                current = word.to_owned();
            }
        }
        result.push(current);
    }
    result
}
