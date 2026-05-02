//! Text-paint resolution.
//!
//! Owns layout (font metrics, line wrap, per-line positioning). The output
//! `TextPaint` is the renderer-ready snapshot consumed by
//! `CanvasRenderer::paint_text`. Per the `*Paint` convention, every
//! allocation that depends on cell content lives here, not at paint time.

use ironcalc_base::types::{CellType, HorizontalAlignment, Style, VerticalAlignment};
use web_sys::CanvasRenderingContext2d;

use crate::geometry::PixelRect;
use crate::model::{CellAddress, CssColor};
use crate::renderer::CanvasRenderer;
use crate::style::FontStyle;
use crate::theme::CanvasTheme;
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
    pub font_css: String,
    pub font_size_px: f64,
    pub color: String,
    pub underline: bool,
    pub strike: bool,
    pub lines: Vec<TextLine>,
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
        let text = model
            .get_formatted_cell_value(addr.sheet, addr.row, addr.column)
            .ok()?;
        if text.is_empty() {
            return None;
        }
        if rect.width < MIN_TEXT_DIM_PX || rect.height < MIN_TEXT_DIM_PX {
            return None;
        }

        // Destructure to move fields directly - avoids cloning `css`.
        let CellTextStyle {
            font:
                FontStyle {
                    css: font_css,
                    size_px,
                    underline,
                    strikethrough: strike,
                    ..
                },
            text_color,
            h_align,
            v_align,
            wrap_text,
            ..
        } = CellTextStyle::resolve(
            model,
            addr.sheet,
            addr.row,
            addr.column,
            renderer.theme(),
            style,
        );

        let approx_char_w = size_px * CHAR_WIDTH_FACTOR;
        let line_height = size_px * LINE_HEIGHT_FACTOR;
        let usable_w = rect.width - 2.0 * CELL_PADDING;
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
                HorizontalAlignment::Right => right - CELL_PADDING - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => center.x,
                _ => rect.top_left.x + CELL_PADDING + tw / 2.0,
            };
            let center_y = match v_align {
                VerticalAlignment::Bottom => {
                    bottom - size_px / 2.0 - TEXT_V_INSET_PX
                        + (i_f - line_count + 1.0) * line_height
                }
                VerticalAlignment::Center => {
                    center.y + (i_f + (1.0 - line_count) / 2.0) * line_height
                }
                _ => rect.top_left.y + size_px / 2.0 + TEXT_V_INSET_PX + i_f * line_height,
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
            color: CssColor::new(&text_color).0,
            underline,
            strike,
            lines,
        })
    }
}

/// Per-cell text styling resolved from the model's raw `Style`. Private step
/// inside `TextPaint::resolve`; not exported.
struct CellTextStyle {
    text_color: String,
    font: FontStyle,
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

        let text_color = match style.font.color.as_deref() {
            None | Some("#000000") => CssColor::new(theme.default_text_color),
            Some(c) => CssColor::new(c),
        };

        let size_px = style.font.sz as f64;
        // Fallback to default as in IronCalc Font name default.
        let css = FontStyle::build(
            size_px,
            style.font.b,
            style.font.i,
            &style.font.name,
            "Calibri",
        );
        let font = FontStyle {
            size_px,
            underline: style.font.u,
            strikethrough: style.font.strike,
            css,
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
            text_color: text_color.0,
            font,
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
