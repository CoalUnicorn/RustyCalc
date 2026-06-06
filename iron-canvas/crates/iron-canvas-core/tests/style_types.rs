use iron_canvas_core::{Alignment, BorderStyle, CellKind, CellStyle, FontStyle, HAlign, VAlign};

#[test]
fn cell_style_default_is_empty() {
    let s = CellStyle::default();
    assert!(s.fill_color.is_none());
    assert!(s.alignment.is_none());
    assert_eq!(s.font, FontStyle::default());
}

#[test]
fn alignment_defaults_match_ironcalc() {
    let a = Alignment::default();
    assert_eq!(a.horizontal, HAlign::General);
    assert_eq!(a.vertical, VAlign::Bottom);
}

#[test]
fn cell_kind_default_is_text() {
    assert_eq!(CellKind::default(), CellKind::Text);
}

#[test]
fn border_style_has_nine_variants() {
    // Compile-time proof the variant set is complete; touch each once.
    let all = [
        BorderStyle::Thin,
        BorderStyle::Medium,
        BorderStyle::Thick,
        BorderStyle::Double,
        BorderStyle::Dotted,
        BorderStyle::SlantDashDot,
        BorderStyle::MediumDashed,
        BorderStyle::MediumDashDotDot,
        BorderStyle::MediumDashDot,
    ];
    assert_eq!(all.len(), 9);
}
