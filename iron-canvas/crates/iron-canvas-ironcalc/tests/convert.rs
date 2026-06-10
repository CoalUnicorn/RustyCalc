use iron_canvas_core::{BorderStyle, CellKind, HAlign, VAlign};
use iron_canvas_ironcalc::convert::{
    alignment_to_core, border_style_to_core, border_to_core, cell_type_to_kind, color_to_css,
    font_to_core, halign_to_core, style_to_core, valign_to_core,
};
use ironcalc_base::types as ic;

/// Test resolver over the default (Office) theme — same shape the live call
/// sites build over `UserModel::resolve_color`.
fn office_resolver(theme: &ic::Theme) -> impl Fn(&ic::Color) -> Option<String> + '_ {
    move |c| color_to_css(c, theme)
}

#[test]
fn font_maps_field_for_field() {
    let theme = ic::Theme::default();
    let f = ic::Font {
        b: true,
        sz: 14,
        name: "Inter".into(),
        color: ic::Color::Rgb("#ff0000".into()),
        ..ic::Font::default()
    };
    let core = font_to_core(f, &office_resolver(&theme));
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
    let theme = ic::Theme::default();
    let b = ic::Border {
        diagonal_up: true,
        diagonal_down: true,
        ..ic::Border::default()
    };
    let core = border_to_core(b, &office_resolver(&theme));
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
    let theme = ic::Theme::default();
    let s = ic::Style {
        fill: ic::Fill {
            color: ic::Color::Rgb("#eeeeee".into()),
        },
        ..ic::Style::default()
    };
    let core = style_to_core(s, &office_resolver(&theme));
    assert_eq!(core.fill_color.as_deref(), Some("#eeeeee"));
}

#[test]
fn theme_color_resolves_through_workbook_theme() {
    let theme = ic::Theme::default();
    // Slot 4 = accent1; tint 0.0 is identity, so the Office hex comes back.
    assert_eq!(
        color_to_css(&ic::Color::Theme(4, 0.0), &theme).as_deref(),
        Some("#4472C4")
    );
    // Color::None resolves to nothing, not an empty CSS string.
    assert_eq!(color_to_css(&ic::Color::None, &theme), None);
}

#[test]
fn update_range_style_writes_theme_and_empty_clears() {
    use iron_canvas_ironcalc::color_resolver;
    use ironcalc_base::UserModel;
    use ironcalc_base::expressions::types::Area;

    let mut m = match UserModel::new_empty("wb", "en", "UTC", "en") {
        Ok(m) => m,
        Err(e) => panic!("empty workbook must construct: {e}"),
    };
    let area = Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 1,
        height: 1,
    };

    // "[idx, tint]" goes through Color::from_param → Color::Theme; the
    // resolver (borrowing the workbook theme) must yield the accent1 hex.
    if let Err(e) = m.update_range_style(&area, "fill.color", "[4, 0]") {
        panic!("theme fill write failed: {e}");
    }
    let style = match m.get_cell_style(0, 1, 1) {
        Ok(s) => s,
        Err(e) => panic!("cell style read failed: {e}"),
    };
    let core = style_to_core(style, &color_resolver(&m));
    assert_eq!(core.fill_color.as_deref(), Some("#4472C4"));

    // RustyCalc's clear path passes "" — from_param maps it to Color::None,
    // which must convert to no fill, not an empty CSS string.
    if let Err(e) = m.update_range_style(&area, "fill.color", "") {
        panic!("empty-string clear failed: {e}");
    }
    let style = match m.get_cell_style(0, 1, 1) {
        Ok(s) => s,
        Err(e) => panic!("cell style re-read failed: {e}"),
    };
    let core = style_to_core(style, &color_resolver(&m));
    assert_eq!(core.fill_color, None);
}

#[test]
fn theme_font_color_survives_conversion() {
    let theme = ic::Theme::default();
    let f = ic::Font {
        color: ic::Color::Theme(5, 0.0), // accent2
        ..ic::Font::default()
    };
    let core = font_to_core(f, &office_resolver(&theme));
    assert_eq!(
        core.color.as_deref(),
        Some("#ED7D31"),
        "theme colors must resolve at the model boundary — dropping them to \
         None loses fills/fonts/borders on most real xlsx files"
    );
}
