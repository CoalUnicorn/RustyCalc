//! Font CSS-string builders.
//!
//! `FontStyle` carries a pre-built canvas `ctx.font` string plus the
//! decoration flags (underline, strike) the painter needs separately.
//! Cell-level resolution (alignment, colour) lives in `crate::types`.

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

pub(crate) fn escape_font_family(name: &str, fallback: &str) -> String {
    match name.trim() {
        "" => fallback.to_owned(),
        n if n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => n.to_owned(),
        n => format!("\"{}\"", n.replace('"', "")),
    }
}
