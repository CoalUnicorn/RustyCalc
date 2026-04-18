//! Cell text layout (Phase 1 collect) and paint (Phase 4).
//!
//! Text is deferred to a final paint phase so it renders on top of cell
//! backgrounds, selection fill, and header strokes. `compute_cell_text`
//! builds a `CellText` with pre-measured, pre-positioned lines; Phase 4
//! hands each one to `render_cell_text`.

use ironcalc_base::types::{HorizontalAlignment, VerticalAlignment};
use ironcalc_base::UserModel;

use crate::model::frontend_model::FrontendModel;

use super::super::geometry::PixelRect;
use super::super::types::{CellText, TextLine};
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

//  Text-layout tuning constants

pub(super) const CELL_PADDING: f64 = 4.0;
pub(super) const DEFAULT_FONT_FAMILY: &str = "Inter, Arial, sans-serif";
const UNDERLINE_OFFSET_FACTOR: f64 = 0.35;
const MIN_UNDERLINE_OFFSET: f64 = 2.0;
const CHAR_WIDTH_FACTOR: f64 = 0.6;
const LINE_HEIGHT_FACTOR: f64 = 1.5;

impl CanvasRenderer {
    /// Build the text layout for a cell; returns `None` for empty or
    /// too-small cells.
    pub(super) fn compute_cell_text(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        col: i32,
        rect: PixelRect,
    ) -> Option<CellText> {
        let PixelRect { x, y, width, height } = rect;

        if width <= 0.0 || height <= 0.0 || !self.is_rect_visible(rect) {
            return None;
        }

        let text = model.get_formatted_cell_value(sheet, row, col).ok()?;
        if text.is_empty() {
            return None;
        }
        // Below this size, even a single glyph would overflow the cell.
        if width < 10.0 || height < 10.0 {
            return None;
        }

        // Destructure to move fields directly — avoids cloning `css`.
        let crate::model::ResolvedCellStyle {
            font:
                crate::model::ResolvedFont {
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
        } = model.cell_style(sheet, row, col, self.theme.default_text_color);

        let approx_char_w = font_size * CHAR_WIDTH_FACTOR;
        let line_height = font_size * LINE_HEIGHT_FACTOR;
        let usable_w = width - 2.0 * CELL_PADDING;

        // Set font on ctx so measure_text() returns accurate widths.
        self.ctx.set_font(&font);

        let text_lines: Vec<String> = if wrap && usable_w > 0.0 {
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
        } else {
            text.split('\n').map(str::to_owned).collect()
        };

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
                HorizontalAlignment::Right => width - CELL_PADDING + x - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    x + width / 2.0
                }
                _ => CELL_PADDING + x + tw / 2.0,
            };
            let center_y = match effective_v_align {
                VerticalAlignment::Bottom => {
                    y + height - font_size / 2.0 - 4.0 + (i_f - line_count + 1.0) * line_height
                }
                VerticalAlignment::Center => {
                    y + height / 2.0 + (i_f + (1.0 - line_count) / 2.0) * line_height
                }
                _ => y + font_size / 2.0 + 4.0 + i_f * line_height,
            };
            lines.push(TextLine {
                text: line,
                center_x,
                center_y,
                width: tw,
            });
        }

        Some(CellText {
            clip: PixelRect { x, y, width, height },
            font,
            font_size_px: font_size,
            text_color,
            underlined,
            strike,
            lines,
        })
    }

    /// Paint a pre-computed `CellText` onto the canvas.
    pub(super) fn render_cell_text(&self, ct: &CellText) {
        let ctx = &self.ctx;
        ctx.set_font(&ct.font);
        ctx.set_fill_style_str(ct.text_color.as_str());

        ctx.save();
        ctx.begin_path();
        ctx.rect(ct.clip.x, ct.clip.y, ct.clip.width, ct.clip.height);
        ctx.clip();

        for line in &ct.lines {
            ctx.fill_text(&line.text, line.center_x, line.center_y).ok();
            if ct.underlined {
                let underline_offset =
                    (ct.font_size_px * UNDERLINE_OFFSET_FACTOR).max(MIN_UNDERLINE_OFFSET);
                ctx.begin_path();
                ctx.set_stroke_style_str(ct.text_color.as_str());
                ctx.set_line_width(STANDARD_BORDER_WIDTH);
                ctx.move_to(
                    line.center_x - line.width / 2.0,
                    line.center_y + underline_offset,
                );
                ctx.line_to(
                    line.center_x + line.width / 2.0,
                    line.center_y + underline_offset,
                );
                ctx.stroke();
            }
            if ct.strike {
                ctx.begin_path();
                ctx.set_stroke_style_str(ct.text_color.as_str());
                ctx.set_line_width(STANDARD_BORDER_WIDTH);
                ctx.move_to(line.center_x - line.width / 2.0, line.center_y);
                ctx.line_to(line.center_x + line.width / 2.0, line.center_y);
                ctx.stroke();
            }
        }
        ctx.restore();
    }
}
