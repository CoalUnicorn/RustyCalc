use iron_canvas_core::{BorderStyle, CellKind, HAlign, VAlign};
use iron_canvas_ironcalc::convert::{
    alignment_to_core, border_style_to_core, border_to_core, cell_type_to_kind, font_to_core,
    halign_to_core, style_to_core, valign_to_core,
};
use ironcalc_base::types as ic;

#[test]
fn font_maps_field_for_field() {
    let mut f = ic::Font::default();
    f.b = true;
    f.sz = 14;
    f.name = "Inter".into();
    f.color = Some("#ff0000".into());
    let core = font_to_core(f);
    assert!(core.bold);
    assert_eq!(core.size, 14.0);
    assert_eq!(core.name, "Inter");
    assert_eq!(core.color.as_deref(), Some("#ff0000"));
}

#[test]
fn cell_type_collapses_to_kind() {
    assert_eq!(cell_type_to_kind(ic::CellType::Number), CellKind::Number);
    assert_eq!(cell_type_to_kind(ic::CellType::ErrorValue), CellKind::Error);
    assert_eq!(cell_type_to_kind(ic::CellType::Array), CellKind::Text);
    assert_eq!(cell_type_to_kind(ic::CellType::Text), CellKind::Text);
}

#[test]
fn alignment_and_border_style_round_trip() {
    assert_eq!(
        halign_to_core(ic::HorizontalAlignment::Right),
        HAlign::Right
    );
    assert_eq!(valign_to_core(ic::VerticalAlignment::Top), VAlign::Top);
    assert_eq!(
        border_style_to_core(ic::BorderStyle::MediumDashDot),
        BorderStyle::MediumDashDot
    );
}

#[test]
fn border_diagonal_flags_carry_through() {
    let b = ic::Border {
        diagonal_up: true,
        diagonal_down: true,
        ..ic::Border::default()
    };
    let core = border_to_core(b);
    assert!(core.diagonal_up);
    assert!(core.diagonal_down);
}

#[test]
fn alignment_carries_wrap_text() {
    let a = ic::Alignment {
        wrap_text: true,
        ..ic::Alignment::default()
    };
    assert!(
        alignment_to_core(a).wrap_text,
        "wrap_text must survive conversion — the renderer reads it for word-wrap, \
         autofit row height, and the paint-skip fingerprint"
    );
}

#[test]
fn style_fill_color_flattens() {
    let mut s = ic::Style::default();
    s.fill.color = Some("#eee".into());
    let core = style_to_core(s);
    assert_eq!(core.fill_color.as_deref(), Some("#eee"));
}
