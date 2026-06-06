//! IronCalc → core styling conversions. Pure value maps — no theme needed
//! (colors are CSS strings; the renderer applies theme defaults at paint time).
//!
//! `From` impls are impossible here because both sides are foreign types
//! (orphan rule). Free functions serve the same role for the bridge crate.

use iron_canvas_core::{
    Alignment, Border, BorderItem, BorderStyle, CellDecoration, CellKind, CellStyle, DataBarSpec,
    FontStyle, HAlign, RatingSpec, VAlign,
};
use ironcalc_base::cf_types as ic_cf;
use ironcalc_base::types as ic;

pub fn style_to_core(s: ic::Style) -> CellStyle {
    CellStyle {
        fill_color: s.fill.color,
        font: font_to_core(s.font),
        alignment: s.alignment.map(alignment_to_core),
        border: border_to_core(s.border),
    }
}

pub fn font_to_core(f: ic::Font) -> FontStyle {
    FontStyle {
        name: f.name,
        size: f64::from(f.sz),
        color: f.color,
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

pub fn border_to_core(b: ic::Border) -> Border {
    // core has no diagonal BorderItem slot; the two direction flags carry through.
    Border {
        left: b.left.map(border_item_to_core),
        right: b.right.map(border_item_to_core),
        top: b.top.map(border_item_to_core),
        bottom: b.bottom.map(border_item_to_core),
        diagonal_up: b.diagonal_up,
        diagonal_down: b.diagonal_down,
    }
}

pub fn border_item_to_core(b: ic::BorderItem) -> BorderItem {
    BorderItem {
        style: border_style_to_core(b.style),
        color: b.color,
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
/// when no decoration applies (icon → data_bar → rating priority order).
///
/// IconSpec is a String placeholder — `Debug` of `Icon` gives the variant
/// name ("Circle", "ArrowUp", …), which is fine for a no-op decoration.
pub fn cell_decoration_from_extended(ext: &ic_cf::ExtendedStyle) -> Option<CellDecoration> {
    if let Some(ref icon) = ext.icon {
        return Some(CellDecoration::Icon(format!("{:?}", icon.icon)));
    }
    if let Some(ref bar) = ext.data_bar {
        return Some(CellDecoration::DataBar(DataBarSpec {
            color: bar.positive_color.clone(),
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
