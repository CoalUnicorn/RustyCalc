//! Cell text layout (Phase 1 collect) and paint (Phase 4).
//!
//! Text is deferred to a final paint phase so it renders on top of cell
//! backgrounds, selection fill, and header strokes. `compute_cell_text`
//! builds a `CellText` with pre-measured, pre-positioned lines; Phase 4
//! hands each one to `render_cell_text`.

use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};
use ironcalc_base::UserModel;

use super::super::geometry::PixelRect;
use super::super::types::{CellText, TextLine};
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};
use crate::canvas::Span;
use crate::coord::CellAddress;
use crate::model::frontend_model::FrontendModel;
use crate::model::{ResolvedCellStyle, ResolvedFont};

pub(super) const CELL_PADDING: f64 = 4.0;
pub(super) const DEFAULT_FONT_FAMILY: &str = "Inter, Arial, sans-serif";
// With textBaseline:"middle", center_y is the em-square midpoint. The
// typographic baseline sits at ~center_y + font_size*0.15; 0.35x puts the
// underline just below the baseline, clear of the glyphs.
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: f64 = 2.0;
const CHAR_WIDTH_FACTOR: f64 = 0.6;
const LINE_HEIGHT_FACTOR: f64 = 1.5;
/// Vertical padding between the cell edge and the first/last line of text.
/// Applied at top-align (rect top) and bottom-align (rect bottom).
const TEXT_V_INSET_PX: f64 = 4.0;

impl CanvasRenderer {
    /// Build the text layout for a cell; returns `None` for empty or
    /// too-small cells.
    pub(super) fn compute_cell_text(
        &self,
        model: &UserModel,
        addr: CellAddress,
        rect: PixelRect,
    ) -> Option<CellText> {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 || !self.is_rect_visible(rect) {
            return None;
        }

        let text = model
            .get_formatted_cell_value(addr.sheet, addr.row, addr.column)
            .ok()?;
        if text.is_empty() {
            return None;
        }
        // Below this size, even a single glyph would overflow the cell.
        if rect.size.x < 10.0 || rect.size.y < 10.0 {
            return None;
        }

        // Destructure to move fields directly — avoids cloning `css`.
        let ResolvedCellStyle {
            font:
                ResolvedFont {
                    css: font,
                    size_px: font_size,
                    underline: underlined,
                    strikethrough: strike,
                    ..
                },
            text_color,
            h_align: effective_h_align,
            v_align: effective_v_align,
            wrap_text: wrap,
            ..
        } = model.cell_style(addr, self.theme.default_text_color);

        let approx_char_w = font_size * CHAR_WIDTH_FACTOR;
        let line_height = font_size * LINE_HEIGHT_FACTOR;
        let usable_w = rect.size.x - 2.0 * CELL_PADDING;
        let right = rect.right();
        let bottom = rect.bottom();
        let center = rect.center();

        // Set font on ctx so measure_text() returns accurate widths.
        self.ctx.set_font(&font);

        let text_lines = self.layout_lines(&text, wrap, usable_w, approx_char_w);

        let line_count = text_lines.len() as f64;
        let mut lines: Vec<TextLine> = Vec::new();

        for (i, line) in text_lines.into_iter().enumerate() {
            let tw = self
                .ctx
                .measure_text(&line)
                .map(|m| m.width())
                .unwrap_or(line.len() as f64 * approx_char_w);
            let i_f = i as f64;
            let center_x = match effective_h_align {
                HorizontalAlignment::Right => right - CELL_PADDING - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => center.x,
                _ => rect.top_left.x + CELL_PADDING + tw / 2.0,
            };
            let center_y = match effective_v_align {
                VerticalAlignment::Bottom => {
                    bottom - font_size / 2.0 - TEXT_V_INSET_PX
                        + (i_f - line_count + 1.0) * line_height
                }
                VerticalAlignment::Center => {
                    center.y + (i_f + (1.0 - line_count) / 2.0) * line_height
                }
                _ => rect.top_left.y + font_size / 2.0 + TEXT_V_INSET_PX + i_f * line_height,
            };
            lines.push(TextLine {
                text: line,
                center_x,
                center_y,
                width: tw,
            });
        }

        Some(CellText {
            clip: rect,
            font,
            font_size_px: font_size,
            text_color,
            underlined,
            strike,
            lines,
        })
    }

    /// Break `text` into render-ready lines: split on `\n` always, then
    /// word-wrap within each split when `wrap` is on and the cell has width.
    ///
    /// `approx_char_w` is the fallback glyph width when `measure_text` fails
    /// (e.g. before the canvas font has resolved); it biases the wrap point
    /// slightly but never loses characters.
    fn layout_lines(
        &self,
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
                let w = self
                    .ctx
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

    /// Paint a pre-computed `CellText` onto the canvas.
    pub(super) fn render_cell_text(&self, ct: &CellText) {
        self.ctx.set_font(&ct.font);
        self.ctx.set_fill_style_str(ct.text_color.as_str());

        self.with_clip(ct.clip, |this| {
            if ct.underlined || ct.strike {
                this.ctx.set_stroke_style_str(ct.text_color.as_str());
                this.ctx.set_line_width(STANDARD_BORDER_WIDTH);
            }
            let underline_offset =
                (ct.font_size_px * UNDERLINE_OFFSET_FACTOR).max(MIN_UNDERLINE_OFFSET);

            for line in &ct.lines {
                this.ctx
                    .fill_text(&line.text, line.center_x, line.center_y)
                    .ok();
                let x1 = line.center_x - line.width / 2.0;
                let x2 = line.center_x + line.width / 2.0;
                if ct.underlined {
                    this.stroke_hline(Span { from: x1, to: x2 }, line.center_y + underline_offset);
                }
                if ct.strike {
                    this.stroke_hline(Span { from: x1, to: x2 }, line.center_y);
                }
            }
        });
    }
}
