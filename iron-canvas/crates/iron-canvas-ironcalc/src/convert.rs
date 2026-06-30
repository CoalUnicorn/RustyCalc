//! IronCalc -> core styling conversions. Colors are no longer self-contained
//! (`ic::Color::Theme` needs the workbook theme), so conversions resolve them
//! at this model boundary through a [`ColorResolver`] the caller builds over
//! whatever theme access it already has — `UserModel::resolve_color` borrows
//! the theme, so no per-cell theme clone is ever needed.
//!
//! `From` impls are impossible here because both sides are foreign types
//! (orphan rule). Free functions serve the same role for the bridge crate.

use iron_canvas_core::{
    Alignment, Border, BorderItem, BorderStyle, CellDecoration, CellKind, CellStyle, DataBarSpec,
    FontStyle, HAlign, RatingSpec, VAlign,
};
use ironcalc_base::cf_types as ic_cf;
use ironcalc_base::types as ic;

/// Maps an IronCalc color to a CSS `#RRGGBB` string, `None` for `Color::None`.
pub type ColorResolver<'a> = &'a dyn Fn(&ic::Color) -> Option<String>;

/// Resolve against an explicit theme — for callers that hold a `Theme` rather
/// than a `UserModel` (tests, the iron-canvas-web mirror).
pub fn color_to_css(c: &ic::Color, theme: &ic::Theme) -> Option<String> {
    let rgb = c.to_rgb(theme);
    (!rgb.is_empty()).then_some(rgb)
}

pub fn style_to_core(s: ic::Style, resolve: ColorResolver) -> CellStyle {
    CellStyle {
        fill_color: resolve(&s.fill.color),
        font: font_to_core(s.font, resolve),
        alignment: s.alignment.map(alignment_to_core),
        border: border_to_core(s.border, resolve),
    }
}

pub fn font_to_core(f: ic::Font, resolve: ColorResolver) -> FontStyle {
    FontStyle {
        name: f.name,
        size: f64::from(f.sz),
        color: resolve(&f.color),
        bold: f.b,
        italic: f.i,
        underline: f.u,
        strike: f.strike,
    }
}

pub fn alignment_to_core(a: ic::Alignment) -> Alignment {
    Alignment {
        horizontal: halign_to_core(a.horizontal),
        vertical: valign_to_core(a.vertical),
        wrap_text: a.wrap_text,
    }
}

pub fn halign_to_core(h: ic::HorizontalAlignment) -> HAlign {
    use ic::HorizontalAlignment as I;
    match h {
        I::Center => HAlign::Center,
        I::CenterContinuous => HAlign::CenterContinuous,
        I::Distributed => HAlign::Distributed,
        I::Fill => HAlign::Fill,
        I::General => HAlign::General,
        I::Justify => HAlign::Justify,
        I::Left => HAlign::Left,
        I::Right => HAlign::Right,
    }
}

pub fn valign_to_core(v: ic::VerticalAlignment) -> VAlign {
    use ic::VerticalAlignment as I;
    match v {
        I::Bottom => VAlign::Bottom,
        I::Center => VAlign::Center,
        I::Distributed => VAlign::Distributed,
        I::Justify => VAlign::Justify,
        I::Top => VAlign::Top,
    }
}

pub fn border_to_core(b: ic::Border, resolve: ColorResolver) -> Border {
    // core has no diagonal BorderItem slot; the two direction flags carry through.
    Border {
        left: b.left.map(|i| border_item_to_core(i, resolve)),
        right: b.right.map(|i| border_item_to_core(i, resolve)),
        top: b.top.map(|i| border_item_to_core(i, resolve)),
        bottom: b.bottom.map(|i| border_item_to_core(i, resolve)),
        diagonal_up: b.diagonal_up,
        diagonal_down: b.diagonal_down,
    }
}

pub fn border_item_to_core(b: ic::BorderItem, resolve: ColorResolver) -> BorderItem {
    BorderItem {
        style: border_style_to_core(b.style),
        color: resolve(&b.color),
    }
}

pub fn border_style_to_core(s: ic::BorderStyle) -> BorderStyle {
    use ic::BorderStyle as I;
    match s {
        I::Thin => BorderStyle::Thin,
        I::Medium => BorderStyle::Medium,
        I::Thick => BorderStyle::Thick,
        I::Double => BorderStyle::Double,
        I::Dotted => BorderStyle::Dotted,
        I::SlantDashDot => BorderStyle::SlantDashDot,
        I::MediumDashed => BorderStyle::MediumDashed,
        I::MediumDashDotDot => BorderStyle::MediumDashDotDot,
        I::MediumDashDot => BorderStyle::MediumDashDot,
    }
}

/// Map an IronCalc `ExtendedStyle` to a core `CellDecoration`. Returns `None`
/// when no decoration applies (icon -> data_bar -> rating priority order).
///
/// IconSpec is a String placeholder — `Debug` of `Icon` gives the variant
/// name ("Circle", "ArrowUp", ...), which is fine for a no-op decoration.
pub fn cell_decoration_from_extended(
    ext: &ic_cf::ExtendedStyle,
    resolve: ColorResolver,
) -> Option<CellDecoration> {
    if let Some(ref icon) = ext.icon {
        return Some(CellDecoration::Icon(format!("{:?}", icon.icon)));
    }
    if let Some(ref bar) = ext.data_bar {
        // Color::None falls back to Excel's default data-bar blue — an
        // uncolored bar should still be visible, not an empty CSS string.
        let color = resolve(&bar.positive_color).unwrap_or_else(|| "#638EC6".into());
        return Some(CellDecoration::DataBar(DataBarSpec {
            color,
            fraction: bar.value.clamp(0.0, 1.0),
        }));
    }
    if let Some(ref rating) = ext.rating {
        // CfRating.count/max are u32; RatingSpec is u32 — no cast needed.
        return Some(CellDecoration::Rating(RatingSpec {
            stars: rating.max,
            filled: rating.count,
        }));
    }
    None
}

pub fn cell_type_to_kind(t: ic::CellType) -> CellKind {
    use ic::CellType as I;
    match t {
        I::Number => CellKind::Number,
        I::ErrorValue => CellKind::Error,
        I::LogicalValue => CellKind::Logical,
        // Array and CompoundData have no dedicated core kind; treat as text.
        I::Text | I::Array | I::CompoundData => CellKind::Text,
    }
}
