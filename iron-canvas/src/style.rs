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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_family_returns_fallback() {
        assert_eq!(escape_font_family("", "Calibri"), "Calibri");
    }

    #[test]
    fn whitespace_only_family_returns_fallback() {
        assert_eq!(escape_font_family("   ", "Calibri"), "Calibri");
    }

    #[test]
    fn alphanumeric_family_passes_through_bare() {
        assert_eq!(escape_font_family("Arial", "Calibri"), "Arial");
        assert_eq!(escape_font_family("Helvetica-9", "Calibri"), "Helvetica-9");
    }

    #[test]
    fn family_with_space_gets_quoted() {
        assert_eq!(
            escape_font_family("Times New Roman", "Calibri"),
            "\"Times New Roman\""
        );
    }

    #[test]
    fn embedded_quotes_are_stripped_before_wrapping() {
        // Defensive: a font-family literal with a `"` inside it would break
        // the CSS font-string. Strip and re-wrap.
        assert_eq!(
            escape_font_family("Arial\"; }evil:1; \"", "Calibri"),
            "\"Arial; }evil:1; \""
        );
    }

    #[test]
    fn build_emits_size_and_family_only_when_plain() {
        assert_eq!(
            FontStyle::build(12.0, false, false, "Arial", "Calibri"),
            "12px Arial"
        );
    }

    #[test]
    fn build_prefixes_bold_then_italic() {
        assert_eq!(
            FontStyle::build(14.0, true, true, "Arial", "Calibri"),
            "bold italic 14px Arial"
        );
    }

    #[test]
    fn build_falls_back_to_default_family_when_blank() {
        assert_eq!(
            FontStyle::build(10.0, false, false, "", "Calibri"),
            "10px Calibri"
        );
    }
}
