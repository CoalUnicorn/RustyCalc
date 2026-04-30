use crate::style::{escape_font_family, FontStyle};

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
