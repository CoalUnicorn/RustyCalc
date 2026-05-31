//! Pure CSS-string construction for the Canvas-2D `ctx.font` field.
//!
//! Lives next to [`super::intern::FontIntern`] — the intern table is the
//! only consumer. Split out so the formatting concern (`"bold italic 14px
//! Helvetica"`) stays separable from the deduplication concern (linear
//! scan into `Rc<str>`).

/// Format `(size, weight, slant, family)` into the `ctx.font` CSS string
/// the canvas backend expects. Family is run through [`escape_font_family`]
/// so user-supplied themes can't corrupt the string with stray quotes.
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

/// CSS-safe rendering of a font family name. Empty / whitespace-only
/// names fall back to `fallback`. Plain ASCII identifiers (alphanumeric
/// plus `-`) pass through unquoted. Anything else is wrapped in double
/// quotes with embedded `"` stripped.
pub(crate) fn escape_font_family(name: &str, fallback: &str) -> String {
    match name.trim() {
        "" => fallback.to_owned(),
        n if n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => n.to_owned(),
        n => format!("\"{}\"", n.replace('"', "")),
    }
}
