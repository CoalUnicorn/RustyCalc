//! IronCalc -> core styling conversions. Colors are no longer self-contained
//! (`ic::Color::Theme` needs the workbook theme), so conversions resolve them
//! at this model boundary through a [`ColorResolver`] the caller builds over
//! whatever theme access it already has — `UserModel::resolve_color` borrows
//! the theme, so no per-cell theme clone is ever needed.
//!
//! `From` impls are impossible here because both sides are foreign types
//! (orphan rule). Free functions serve the same role for the bridge crate.

use iron_canvas_core::{
    Alignment, Border, BorderItem, BorderStyle, CellDecoration, CellKind, CellLink, CellStyle,
    DataBarSpec, FontStyle, HAlign, IconGlyph, IconSpec, LinkTarget, RCRange, RatingSpec,
    RatingStyle, VAlign,
};
use ironcalc_base::cf_types as ic_cf;
use ironcalc_base::types as ic;

/// Maps an IronCalc color to a CSS `#RRGGBB` string, `None` for `Color::None`.
pub type ColorResolver<'a> = &'a dyn Fn(&ic::Color) -> Option<String>;

/// Resolve against an explicit theme — for callers that hold a `Theme` rather
/// than a `UserModel` (tests, and the iron-canvas-web JS-backed path).
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

/// Map an IronCalc `Icon` to the core glyph identity. Total: the two enums
/// carry the same 20 variants.
pub fn icon_glyph_from_ic(icon: ic_cf::Icon) -> IconGlyph {
    use ic_cf::Icon as I;
    match icon {
        I::ArrowUp => IconGlyph::ArrowUp,
        I::ArrowRight => IconGlyph::ArrowRight,
        I::ArrowDown => IconGlyph::ArrowDown,
        I::ArrowAngleUp => IconGlyph::ArrowAngleUp,
        I::ArrowAngleDown => IconGlyph::ArrowAngleDown,
        I::Circle => IconGlyph::Circle,
        I::TriangleUp => IconGlyph::TriangleUp,
        I::TriangleDown => IconGlyph::TriangleDown,
        I::TriangleUpFilled => IconGlyph::TriangleUpFilled,
        I::TriangleDownFilled => IconGlyph::TriangleDownFilled,
        I::FlatRectangle => IconGlyph::FlatRectangle,
        I::Rhombus => IconGlyph::Rhombus,
        I::Flag => IconGlyph::Flag,
        I::Check => IconGlyph::Check,
        I::Cross => IconGlyph::Cross,
        I::Exclamation => IconGlyph::Exclamation,
        I::Star => IconGlyph::Star,
        I::Heart => IconGlyph::Heart,
        I::ThumbsUp => IconGlyph::ThumbsUp,
        I::ThumbsDown => IconGlyph::ThumbsDown,
    }
}

/// Map the three evaluated decoration categories to a core `CellDecoration`.
/// Each category is converted independently so a cell that carries more than
/// one keeps them all. Returns `None` when none applies.
pub fn cell_decoration_from_parts(
    icon: Option<&ic_cf::CfIcon>,
    data_bar: Option<&ic_cf::CfDataBar>,
    rating: Option<&ic_cf::CfRating>,
    resolve: ColorResolver,
) -> Option<CellDecoration> {
    let decoration = CellDecoration {
        icon: icon.map(|i| IconSpec {
            glyph: icon_glyph_from_ic(i.icon.clone()),
            color: resolve(&i.color),
            show_value: i.show_value,
        }),
        data_bar: data_bar.map(|bar| DataBarSpec {
            // Color::None falls back to Excel's default data-bar blue — an
            // uncolored bar should still be visible, not an empty CSS string.
            positive_color: resolve(&bar.positive_color).unwrap_or_else(|| "#638EC6".into()),
            negative_color: resolve(&bar.negative_color),
            is_gradient: bar.is_gradient,
            value: bar.value.clamp(0.0, 1.0),
            axis_position: bar.axis_position.clamp(0.0, 1.0),
            show_value: bar.show_value,
        }),
        rating: rating.map(|r| RatingSpec {
            style: match &r.icon {
                ic_cf::Icon::Circle => RatingStyle::FiveQuarters,
                icon => RatingStyle::RepeatedGlyph(icon_glyph_from_ic(icon.clone())),
            },
            color: resolve(&r.color),
            count: r.count,
            max: r.max,
            show_value: r.show_value,
        }),
    };
    (!decoration.is_empty()).then_some(decoration)
}

/// Map an IronCalc `ExtendedStyle` to a core `CellDecoration`.
pub fn cell_decoration_from_extended(
    ext: &ic_cf::ExtendedStyle,
    resolve: ColorResolver,
) -> Option<CellDecoration> {
    cell_decoration_from_parts(
        ext.icon.as_ref(),
        ext.data_bar.as_ref(),
        ext.rating.as_ref(),
        resolve,
    )
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

/// Resolve the workbook's hyperlink theme color.
///
/// The index `10` mirrors `links::THEME_COLOR_HYPERLINK`, which is
/// `pub(crate)` upstream, so it cannot be imported. A test in this crate
/// compares the result with the color a static link's own cell style
/// resolves, which guards the literal against upstream drift.
pub fn hyperlink_color(resolve: ColorResolver) -> Option<String> {
    resolve(&ic::Color::Theme(10, 0.0))
}

/// Convert one IronCalc link into a core [`CellLink`].
///
/// `row` and `column` are the link's 1-based address; `dynamic` is the
/// engine's flag — a formula (`HYPERLINK`) owns a dynamic link.
pub fn link_to_core(
    link: ic::Link,
    row: i32,
    column: i32,
    dynamic: bool,
    resolve: ColorResolver,
) -> CellLink {
    let (target, tooltip) = match link {
        ic::Link::External { target, tooltip } => (LinkTarget::External(target), tooltip),
        ic::Link::Internal { location, tooltip } => (LinkTarget::Internal(location), tooltip),
    };
    CellLink::new(
        RCRange::from_cell(row, column),
        target,
        tooltip,
        dynamic,
        hyperlink_color(resolve),
    )
}

/// Convert one IronCalc merged cell into a core inclusive [`RCRange`].
///
/// Returns `None` when the engine's shape cannot become geometry: a
/// non-positive `width`/`height`, or inclusive bounds that overflow `i32`.
/// The engine guarantees both are valid; a custom model does not, and an
/// invalid rectangle must never reach the slot math or the merge table.
pub fn merged_range_to_core(m: ic::MergedCell) -> Option<RCRange> {
    if m.width <= 0 || m.height <= 0 {
        return None;
    }
    let r2 = m.row.checked_add(m.height - 1)?;
    let c2 = m.column.checked_add(m.width - 1)?;
    Some(RCRange {
        r1: m.row,
        c1: m.column,
        r2,
        c2,
    })
}
