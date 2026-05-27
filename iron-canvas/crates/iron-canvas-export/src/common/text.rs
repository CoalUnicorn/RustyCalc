// Matches RecorderPainter's fallback so wrap math is consistent across
// non-browser backends with no host-side text measurement API.
pub const CHAR_WIDTH_FACTOR: f64 = 1.0;

/// Default size when the CSS font shorthand carries no `<n>px` token.
/// Matches `SvgPainter::parse_font`.
pub const DEFAULT_FONT_SIZE_PX: f64 = 12.0;

/// Extract the first `<n>px` token from a CSS `font` shorthand. Mirrors
/// `SvgPainter::parse_font` but drops the family — the PDF backend has
/// exactly one font (Helvetica). Falls back to [`DEFAULT_FONT_SIZE_PX`]
/// when no `px` token is present.
pub fn parse_font_size_px(font_css: &str) -> f64 {
    for tok in font_css.split_whitespace() {
        if let Some(n) = tok.strip_suffix("px").and_then(|n| n.parse::<f64>().ok()) {
            return n;
        }
    }
    DEFAULT_FONT_SIZE_PX
}
