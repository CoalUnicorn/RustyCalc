use ironcalc_base::types::{CellType, HorizontalAlignment, VerticalAlignment};

use crate::{model::CssColor, theme::CanvasTheme, CanvasModel};

pub struct CellStyle {
    pub text_color: String, // CSS color
    pub font: FontStyle,
    pub h_align: HorizontalAlignment, // re-exported from ironcalc_base
    pub v_align: VerticalAlignment,
    pub wrap_text: bool,
}

impl CellStyle {
    pub fn resolve_cell_style(
        model: &dyn CanvasModel,
        sheet: u32,
        row: i32,
        column: i32,
        theme: &CanvasTheme,
    ) -> Self {
        let style = model.get_cell_style(sheet, row, column).unwrap_or_default();
        let cell_type = model
            .get_cell_type(sheet, row, column)
            .unwrap_or(CellType::Text);

        let text_color = match style.font.color.as_deref() {
            None | Some("#000000") => CssColor::new(theme.default_text_color),
            Some(c) => CssColor::new(c),
        };

        let size_px = style.font.sz as f64;
        let bold = style.font.b;
        let italic = style.font.i;
        let family = &style.font.name;
        let css = FontStyle::build(size_px, bold, italic, family, "Arial");
        let font = FontStyle {
            size_px,
            underline: style.font.u,
            strikethrough: style.font.strike,
            css,
        };

        // Alignment
        let alignment = style.alignment.as_ref();
        let h_align = match alignment.map(|a| &a.horizontal) {
            Some(HorizontalAlignment::Right) => HorizontalAlignment::Right,
            Some(HorizontalAlignment::Center) | Some(HorizontalAlignment::CenterContinuous) => {
                HorizontalAlignment::Center
            }
            Some(HorizontalAlignment::Left) | Some(HorizontalAlignment::Fill) => {
                HorizontalAlignment::Left
            }
            Some(HorizontalAlignment::Justify) | Some(HorizontalAlignment::Distributed) => {
                // Canvas 2D has no justify/distributed - fall back to left.
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

/// Pre-built font parameters for canvas `ctx.font`.
#[derive(Debug, Clone)]
pub struct FontStyle {
    pub css: String, // e.g. "bold italic 12px Arial"
    pub size_px: f64,
    pub underline: bool,
    pub strikethrough: bool,
}

impl FontStyle {
    pub(crate) fn build(
        size_px: f64,
        bold: bool,
        italic: bool,
        family: &str,
        fallback: &str,
    ) -> String {
        let b = if bold { "bold " } else { "" };
        let i = if italic { "italic " } else { "" };
        let safe_family = escape_font_family(family, fallback);
        format!("{b}{i}{size_px}px {safe_family}")
    }
}

fn escape_font_family(name: &str, fallback: &str) -> String {
    match name.trim() {
        "" => fallback.to_owned(),
        n if n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => n.to_owned(),
        n => format!("\"{}\"", n.replace('"', "")),
    }
}
