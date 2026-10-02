//! Resolved conditional-formatting decoration paints.
//!
//! Cell tier, next to the other `*Paint` records ([`super::paint::CellPaint`],
//! [`super::borders::BorderPaint`], [`super::text::TextPaint`]). The
//! resolve/paint split is the module's contract: `CfDecorationPaint::resolve`
//! is the only step in this module that may allocate. It parses colors,
//! clamps the fractions, derives the gradient shades, and interns every CSS
//! color once per unique RGB triple. `CfDecorationPaint::paint` passes
//! borrowed colors and stack vertices to the backend. Backends may allocate.
//!
//! These are renderer paint records, not wire types: the recorder, SVG, and
//! PDF surfaces serialize `Painter` primitives (`rect_fill`,
//! `rect_fill_hgradient`, `fill_path`), so no backend carries a CF-specific
//! method.

use std::rc::Rc;

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::painter::{PaintColor, Painter};
use crate::renderer::cache::ColorIntern;
use crate::renderer::cache::color::{css_rgb, data_bar_rgb};
use crate::style::{CellDecoration, IconGlyph};

/// Resolved icon decoration: the engine-selected glyph and its resolved,
/// interned color.
#[derive(Debug, Clone, PartialEq)]
pub struct CfIconPaint {
    pub glyph: IconGlyph,
    pub color: Rc<str>,
    /// When false, the painted cell value is hidden (the model value, the
    /// formula bar, and editing are unaffected).
    pub show_value: bool,
}

/// Resolved data bar: interned positive/negative colors plus the lighter
/// shade each gradient runs from at the axis. `value` and `axis_position` are
/// normalized positions in `[0, 1]`; the bar spans them, so it is not
/// necessarily anchored at the cell's left edge.
#[derive(Debug, Clone, PartialEq)]
pub struct CfDataBarPaint {
    pub positive: Rc<str>,
    pub positive_light: Rc<str>,
    pub negative: Rc<str>,
    pub negative_light: Rc<str>,
    pub is_gradient: bool,
    pub value: f64,
    pub axis_position: f64,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// Resolved rating: `count` copies of the selected glyph, out of `max`.
#[derive(Debug, Clone, PartialEq)]
pub struct CfRatingPaint {
    pub glyph: IconGlyph,
    pub color: Rc<str>,
    pub count: u8,
    pub max: u8,
    /// When false, the painted cell value is hidden.
    pub show_value: bool,
}

/// Resolved CF decoration for one cell. The engine resolves each category
/// independently, so all three may be present; `paint` draws them in
/// data-bar -> icon -> rating order (background to foreground).
#[derive(Debug, Clone, PartialEq)]
pub struct CfDecorationPaint {
    pub icon: Option<CfIconPaint>,
    pub data_bar: Option<CfDataBarPaint>,
    pub rating: Option<CfRatingPaint>,
}

impl CfDecorationPaint {
    /// Resolve a core `CellDecoration` into renderer-ready paints, or `None`
    /// when no category applies.
    ///
    /// Takes the decoration by value: the caller owns it (the bulk fetch
    /// hands it over through `Fetched::take_value`), so the colors move
    /// instead of cloning. Every CSS color is interned here — one `format!`
    /// per unique rgb triple per renderer lifetime, not per decorated cell
    /// per frame.
    pub(crate) fn resolve(deco: CellDecoration, intern: &ColorIntern) -> Option<Self> {
        if deco.is_empty() {
            return None;
        }
        Some(CfDecorationPaint {
            icon: deco.icon.map(|icon| CfIconPaint {
                glyph: icon.glyph,
                color: intern.get_rgb(icon.color.as_deref().map(css_rgb).unwrap_or([0, 0, 0])),
                show_value: icon.show_value,
            }),
            data_bar: deco.data_bar.map(|bar| {
                let positive = data_bar_rgb(&bar);
                let negative = bar
                    .negative_color
                    .as_deref()
                    .map(css_rgb)
                    .unwrap_or(DEFAULT_NEGATIVE_RGB);
                CfDataBarPaint {
                    positive: intern.get_rgb(positive),
                    positive_light: intern.get_rgb(lighten(positive, GRADIENT_LIGHTEN)),
                    negative: intern.get_rgb(negative),
                    negative_light: intern.get_rgb(lighten(negative, GRADIENT_LIGHTEN)),
                    is_gradient: bar.is_gradient,
                    value: bar.value.clamp(0.0, 1.0),
                    axis_position: bar.axis_position.clamp(0.0, 1.0),
                    show_value: bar.show_value,
                }
            }),
            rating: deco.rating.map(|rating| CfRatingPaint {
                glyph: rating.glyph,
                color: intern.get_rgb(rating.color.as_deref().map(css_rgb).unwrap_or([0, 0, 0])),
                count: rating.count.min(u32::from(u8::MAX)) as u8,
                max: rating.max.min(u32::from(u8::MAX)) as u8,
                show_value: rating.show_value,
            }),
        })
    }

    /// True when any present category hides the painted cell value. The
    /// model value, the formula bar, and editing are deliberately unaffected:
    /// only the canvas text pass consults this.
    pub(crate) fn hides_value(&self) -> bool {
        self.icon.as_ref().is_some_and(|icon| !icon.show_value)
            || self.data_bar.as_ref().is_some_and(|bar| !bar.show_value)
            || self
                .rating
                .as_ref()
                .is_some_and(|rating| !rating.show_value)
    }

    /// Pixels reserved at `rect`'s left edge for the shown indicator, so the
    /// text pass starts after the icon/rating instead of under it. Zero when
    /// nothing is drawn on the left.
    pub(crate) fn reserved_left(&self, rect: PixelRect) -> i32 {
        if self.hides_value() {
            return 0;
        }
        let Some((slot_left, _, size)) = icon_slot(rect) else {
            return 0;
        };
        let offset = slot_left - rect.left();
        let count = self.glyph_count();
        if count == 0 {
            0
        } else {
            (offset + count * size).min(rect.width)
        }
    }

    fn glyph_count(&self) -> i32 {
        i32::from(self.icon.is_some())
            + self
                .rating
                .as_ref()
                .map_or(0, |rating| i32::from(rating.count))
    }

    /// Paint every present decoration over the already-filled cell `rect`,
    /// purely in `Painter` primitives. The renderer owns CF geometry so the
    /// backend stays primitive-only: data bars become a solid or gradient
    /// rect, icons and ratings become `fill_path` polygons.
    pub(crate) fn paint<P: Painter + ?Sized>(&self, painter: &P, rect: PixelRect) {
        // Background to foreground: the bar must not cover the glyphs.
        if let Some(bar) = &self.data_bar {
            paint_data_bar(painter, rect, bar);
        }
        let Some((mut left, top, size)) = icon_slot(rect) else {
            return;
        };
        let inner = rect.inset(CF_INSET, CF_INSET);
        let needs_clip = left + self.glyph_count() * size > inner.right();
        if needs_clip {
            painter.push_clip(inner);
        }
        if let Some(icon) = &self.icon {
            paint_glyph(painter, icon.glyph, left, top, size, &icon.color);
            left += size;
        }
        if let Some(rating) = &self.rating {
            for i in 0..i32::from(rating.count) {
                let glyph_left = left + i * size;
                if glyph_left >= inner.right() {
                    break;
                }
                paint_glyph(painter, rating.glyph, glyph_left, top, size, &rating.color);
            }
        }
        if needs_clip {
            painter.pop_clip();
        }
    }
}

/// Pixel inset applied to a cell rect before painting a CF decoration, so the
/// decoration never overlaps grid lines or explicit borders.
const CF_INSET: i32 = 2;

/// Width reserved on the left of a decorated cell for an icon-set glyph or a
/// rating. The text layout reserves the same width, so the value never
/// overlaps the indicator.
pub(crate) const CF_ICON_AREA_WIDTH: i32 = 20;

/// Gap between the cell's inner edge and the first icon.
const ICON_LEFT_MARGIN: i32 = 3;

/// How far the gradient's axis-side stop is lightened toward white. Excel's
/// gradient data bars run the same hue from the tip to the axis.
const GRADIENT_LIGHTEN: f64 = 0.6;

/// Excel's default negative-side data-bar color, used when the engine has no
/// resolved negative color.
const DEFAULT_NEGATIVE_RGB: [u8; 3] = [0xff, 0x00, 0x00];

/// Lighten `rgb` toward white by `amount` (0.0 = unchanged, 1.0 = white).
fn lighten(rgb: [u8; 3], amount: f64) -> [u8; 3] {
    std::array::from_fn(|i| {
        let channel = f64::from(rgb[i]);
        (channel + (255.0 - channel) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}

/// The icon/rating slot inside `rect`: `(left, top, size)` in pixels, or
/// `None` when the cell is too small to hold one.
fn icon_slot(rect: PixelRect) -> Option<(i32, i32, i32)> {
    let inner = rect.inset(CF_INSET, CF_INSET);
    if inner.width <= 0 || inner.height <= 0 {
        return None;
    }
    let size = CF_ICON_AREA_WIDTH.min(inner.height);
    if size <= 0 {
        return None;
    }
    Some((
        inner.left() + ICON_LEFT_MARGIN,
        inner.top() + (inner.height - size) / 2,
        size,
    ))
}

/// Paint a data bar: a solid or horizontal-gradient rect spanning the zero
/// axis and the value endpoint, colored by the endpoint's side of the axis.
fn paint_data_bar<P: Painter + ?Sized>(painter: &P, rect: PixelRect, bar: &CfDataBarPaint) {
    let inner = rect.inset(CF_INSET, CF_INSET);
    if inner.width <= 0 || inner.height <= 0 {
        return;
    }
    let pad_y = ((f64::from(inner.height) * 0.15).round() as i32).max(2);
    let bar_height = inner.height - pad_y * 2;
    if bar_height <= 0 {
        return;
    }
    let width = f64::from(inner.width);
    let axis_x = inner.left() + (bar.axis_position * width).round() as i32;
    let value_x = inner.left() + (bar.value * width).round() as i32;
    let positive_side = bar.value >= bar.axis_position;
    let (left, right) = if positive_side {
        (axis_x, value_x)
    } else {
        (value_x, axis_x)
    };
    let bar_width = right - left;
    // Zero-length bar (including a zero value on a split axis): draw nothing.
    if bar_width <= 0 {
        return;
    }
    let bar_rect = PixelRect {
        top_left: Point {
            x: left,
            y: inner.top() + pad_y,
        },
        width: bar_width,
        height: bar_height,
    };
    if bar.is_gradient && bar_width > 1 {
        // Axial side is lighter, the tip keeps the full color.
        let (from, to) = if positive_side {
            (&bar.positive_light, &bar.positive)
        } else {
            (&bar.negative, &bar.negative_light)
        };
        painter.rect_fill_hgradient(
            bar_rect,
            PaintColor::Borrowed(from),
            PaintColor::Borrowed(to),
        );
    } else {
        let solid = if positive_side {
            &bar.positive
        } else {
            &bar.negative
        };
        painter.rect_fill(bar_rect, PaintColor::Borrowed(solid));
    }
}

/// Paint one glyph as filled polygons in the `size`×`size` box at `(left,
/// top)`. Icon geometry is backend-neutral vector work: no font, no glyph
/// asset, no browser-only API.
fn paint_glyph<P: Painter + ?Sized>(
    painter: &P,
    glyph: IconGlyph,
    left: i32,
    top: i32,
    size: i32,
    color: &str,
) {
    if size <= 0 {
        return;
    }
    let canvas = GlyphCanvas {
        painter,
        left: f64::from(left),
        top: f64::from(top),
        size: f64::from(size),
        color,
    };
    match glyph {
        IconGlyph::ArrowUp => {
            canvas.poly(&ARROW_UP_HEAD);
            canvas.poly(&ARROW_UP_SHAFT);
        }
        IconGlyph::ArrowDown => {
            canvas.poly(&ARROW_DOWN_HEAD);
            canvas.poly(&ARROW_DOWN_SHAFT);
        }
        IconGlyph::ArrowRight => {
            canvas.poly(&ARROW_RIGHT_HEAD);
            canvas.poly(&ARROW_RIGHT_SHAFT);
        }
        IconGlyph::ArrowAngleUp => {
            canvas.poly(&ARROW_ANGLE_UP_HEAD);
            canvas.segment((0.06, 0.94), (0.56, 0.44), 0.11);
        }
        IconGlyph::ArrowAngleDown => {
            canvas.poly(&ARROW_ANGLE_DOWN_HEAD);
            canvas.segment((0.06, 0.06), (0.56, 0.56), 0.11);
        }
        IconGlyph::Circle => canvas.circle(0.5),
        IconGlyph::TriangleUp => {
            canvas.segment((0.20, 0.65), (0.50, 0.35), 0.05);
            canvas.segment((0.50, 0.35), (0.80, 0.65), 0.05);
        }
        IconGlyph::TriangleDown => {
            canvas.segment((0.20, 0.35), (0.50, 0.65), 0.05);
            canvas.segment((0.50, 0.65), (0.80, 0.35), 0.05);
        }
        IconGlyph::TriangleUpFilled => {
            canvas.poly(&[(0.5, 0.02), (0.98, 0.98), (0.02, 0.98)]);
        }
        IconGlyph::TriangleDownFilled => {
            canvas.poly(&[(0.02, 0.02), (0.98, 0.02), (0.5, 0.98)]);
        }
        IconGlyph::FlatRectangle => {
            canvas.poly(&[(0.02, 0.36), (0.98, 0.36), (0.98, 0.64), (0.02, 0.64)]);
        }
        IconGlyph::Rhombus => {
            canvas.poly(&[(0.5, 0.02), (0.98, 0.5), (0.5, 0.98), (0.02, 0.5)]);
        }
        IconGlyph::Flag => {
            canvas.poly(&[(0.12, 0.02), (0.24, 0.02), (0.24, 0.98), (0.12, 0.98)]);
            canvas.poly(&[
                (0.24, 0.06),
                (0.96, 0.18),
                (0.78, 0.42),
                (0.96, 0.66),
                (0.24, 0.66),
            ]);
        }
        IconGlyph::Check => {
            canvas.segment((0.12, 0.55), (0.40, 0.85), 0.11);
            canvas.segment((0.40, 0.85), (0.90, 0.16), 0.11);
        }
        IconGlyph::Cross => {
            canvas.segment((0.16, 0.16), (0.84, 0.84), 0.12);
            canvas.segment((0.84, 0.16), (0.16, 0.84), 0.12);
        }
        IconGlyph::Exclamation => {
            canvas.poly(&[(0.42, 0.06), (0.58, 0.06), (0.58, 0.64), (0.42, 0.64)]);
            canvas.poly(&[(0.42, 0.76), (0.58, 0.76), (0.58, 0.94), (0.42, 0.94)]);
        }
        IconGlyph::Star => canvas.star(),
        IconGlyph::Heart => {
            canvas.poly(&[
                (0.5, 0.94),
                (0.08, 0.52),
                (0.04, 0.30),
                (0.16, 0.10),
                (0.36, 0.08),
                (0.5, 0.26),
                (0.64, 0.08),
                (0.84, 0.10),
                (0.96, 0.30),
                (0.92, 0.52),
            ]);
        }
        IconGlyph::ThumbsUp => {
            canvas.poly(&[(0.08, 0.36), (0.26, 0.36), (0.26, 0.94), (0.08, 0.94)]);
            canvas.poly(&[(0.32, 0.44), (0.74, 0.44), (0.74, 0.94), (0.32, 0.94)]);
            canvas.poly(&[(0.44, 0.06), (0.62, 0.06), (0.62, 0.44), (0.44, 0.44)]);
        }
        IconGlyph::ThumbsDown => {
            canvas.poly(&[(0.08, 0.06), (0.26, 0.06), (0.26, 0.64), (0.08, 0.64)]);
            canvas.poly(&[(0.32, 0.06), (0.74, 0.06), (0.74, 0.56), (0.32, 0.56)]);
            canvas.poly(&[(0.44, 0.56), (0.62, 0.56), (0.62, 0.94), (0.44, 0.94)]);
        }
    }
}

// Arrow glyphs: a triangular head plus a rectangular shaft, in a unit box
// (x right, y down).
const ARROW_UP_HEAD: [(f64, f64); 3] = [(0.5, 0.02), (0.98, 0.52), (0.02, 0.52)];
const ARROW_UP_SHAFT: [(f64, f64); 4] = [(0.38, 0.42), (0.62, 0.42), (0.62, 0.98), (0.38, 0.98)];
const ARROW_DOWN_HEAD: [(f64, f64); 3] = [(0.5, 0.98), (0.98, 0.48), (0.02, 0.48)];
const ARROW_DOWN_SHAFT: [(f64, f64); 4] = [(0.38, 0.02), (0.62, 0.02), (0.62, 0.58), (0.38, 0.58)];
const ARROW_RIGHT_HEAD: [(f64, f64); 3] = [(0.98, 0.5), (0.48, 0.02), (0.48, 0.98)];
const ARROW_RIGHT_SHAFT: [(f64, f64); 4] = [(0.02, 0.38), (0.58, 0.38), (0.58, 0.62), (0.02, 0.62)];
const ARROW_ANGLE_UP_HEAD: [(f64, f64); 3] = [(1.0, 0.02), (0.40, 0.06), (0.96, 0.60)];
const ARROW_ANGLE_DOWN_HEAD: [(f64, f64); 3] = [(1.0, 0.98), (0.40, 0.94), (0.96, 0.40)];

/// Maps unit-box coordinates onto the pixel glyph box and fills polygons.
struct GlyphCanvas<'a, P: Painter + ?Sized> {
    painter: &'a P,
    left: f64,
    top: f64,
    size: f64,
    color: &'a str,
}

impl<P: Painter + ?Sized> GlyphCanvas<'_, P> {
    /// Fill the closed polygon whose vertices are unit-box coordinates.
    fn poly(&self, unit: &[(f64, f64)]) {
        const MAX_VERTICES: usize = 32;
        let mut pixels = [Point { x: 0, y: 0 }; MAX_VERTICES];
        let count = unit.len().min(MAX_VERTICES);
        for (out, (u, v)) in pixels.iter_mut().zip(unit.iter()) {
            *out = Point {
                x: (self.left + u * self.size).round() as i32,
                y: (self.top + v * self.size).round() as i32,
            };
        }
        self.painter
            .fill_path(&pixels[..count], PaintColor::Borrowed(self.color));
    }

    /// Fill a thick segment from `a` to `b` (unit coords) with `half`
    /// half-thickness — a quad offset perpendicular to the segment.
    fn segment(&self, a: (f64, f64), b: (f64, f64), half: f64) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            return;
        }
        let (px, py) = (-dy / length * half, dx / length * half);
        self.poly(&[
            (a.0 + px, a.1 + py),
            (b.0 + px, b.1 + py),
            (b.0 - px, b.1 - py),
            (a.0 - px, a.1 - py),
        ]);
    }

    /// Fill a disc of `radius` (a unit-box fraction) centred in the glyph
    /// box, through the painter's native circle primitive. `poly` would
    /// round the arc to integer pixels and show a visible polygon; the
    /// native op keeps the edge smooth at any size or device pixel ratio.
    fn circle(&self, radius: f64) {
        let center = 0.5 * self.size;
        self.painter.fill_circle(
            self.left + center,
            self.top + center,
            radius * self.size,
            PaintColor::Borrowed(self.color),
        );
    }

    /// Fill a five-pointed star inscribed in the unit box.
    fn star(&self) {
        let mut points = [(0.0, 0.0); 10];
        for (k, slot) in points.iter_mut().enumerate() {
            let r = if k % 2 == 0 { 0.5 } else { 0.5 * 0.382 };
            let angle = -std::f64::consts::FRAC_PI_2 + (k as f64) * std::f64::consts::PI / 5.0;
            *slot = (0.5 + r * angle.cos(), 0.5 + r * angle.sin());
        }
        self.poly(&points);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{DataBarSpec, IconSpec, RatingSpec};

    fn bar(color: &str, value: f64, axis: f64, is_gradient: bool) -> DataBarSpec {
        DataBarSpec {
            positive_color: color.to_string(),
            negative_color: None,
            is_gradient,
            value,
            axis_position: axis,
            show_value: true,
        }
    }

    fn icon(glyph: IconGlyph, color: Option<&str>) -> IconSpec {
        IconSpec {
            glyph,
            color: color.map(str::to_string),
            show_value: true,
        }
    }

    #[test]
    fn data_bar_clamps_value_and_normalizes_color() {
        let intern = ColorIntern::new();
        let deco = CellDecoration {
            data_bar: Some(bar("#3366CC", 1.5, 0.0, false)),
            ..CellDecoration::default()
        };
        match CfDecorationPaint::resolve(deco, &intern) {
            Some(CfDecorationPaint {
                data_bar: Some(p), ..
            }) => {
                assert_eq!(&*p.positive, "#3366cc");
                assert_eq!(p.value, 1.0);
            }
            other => panic!("expected a data bar, got {other:?}"),
        }
    }

    #[test]
    fn data_bar_interning_reuses_the_same_color() {
        let intern = ColorIntern::new();
        let deco = || CellDecoration {
            data_bar: Some(bar("#3366CC", 0.5, 0.0, true)),
            ..CellDecoration::default()
        };
        let first = CfDecorationPaint::resolve(deco(), &intern);
        let second = CfDecorationPaint::resolve(deco(), &intern);
        match (first, second) {
            (
                Some(CfDecorationPaint {
                    data_bar: Some(a), ..
                }),
                Some(CfDecorationPaint {
                    data_bar: Some(b), ..
                }),
            ) => {
                assert!(Rc::ptr_eq(&a.positive, &b.positive));
                assert!(Rc::ptr_eq(&a.positive_light, &b.positive_light));
            }
            other => panic!("expected two data bars, got {other:?}"),
        }
    }

    #[test]
    fn gradient_shade_lightens_the_positive_color() {
        let deco = CellDecoration {
            data_bar: Some(bar("#000000", 1.0, 0.0, true)),
            ..CellDecoration::default()
        };
        match CfDecorationPaint::resolve(deco, &ColorIntern::new()) {
            Some(CfDecorationPaint {
                data_bar: Some(p), ..
            }) => {
                assert_eq!(&*p.positive, "#000000");
                assert_eq!(&*p.positive_light, "#999999"); // 0.6 * 255 rounded
            }
            other => panic!("expected a data bar, got {other:?}"),
        }
    }

    #[test]
    fn negative_color_defaults_to_red() {
        let deco = CellDecoration {
            data_bar: Some(bar("#3366cc", 0.0, 0.0, false)),
            ..CellDecoration::default()
        };
        match CfDecorationPaint::resolve(deco, &ColorIntern::new()) {
            Some(CfDecorationPaint {
                data_bar: Some(p), ..
            }) => assert_eq!(&*p.negative, "#ff0000"),
            other => panic!("expected a data bar, got {other:?}"),
        }
    }

    #[test]
    fn rating_maps_count_out_of_max() {
        let deco = CellDecoration {
            rating: Some(RatingSpec {
                glyph: IconGlyph::Star,
                color: None,
                count: 3,
                max: 5,
                show_value: true,
            }),
            ..CellDecoration::default()
        };
        match CfDecorationPaint::resolve(deco, &ColorIntern::new()) {
            Some(CfDecorationPaint {
                rating: Some(p), ..
            }) => {
                assert_eq!(p.glyph, IconGlyph::Star);
                assert_eq!((p.count, p.max), (3, 5));
            }
            other => panic!("expected a rating, got {other:?}"),
        }
    }

    #[test]
    fn icon_keeps_glyph_and_resolves_color() {
        let deco = CellDecoration {
            icon: Some(icon(IconGlyph::ArrowUp, Some("#84cb1f"))),
            ..CellDecoration::default()
        };
        match CfDecorationPaint::resolve(deco, &ColorIntern::new()) {
            Some(CfDecorationPaint { icon: Some(p), .. }) => {
                assert_eq!(p.glyph, IconGlyph::ArrowUp);
                assert_eq!(&*p.color, "#84cb1f");
            }
            other => panic!("expected an icon, got {other:?}"),
        }
    }

    #[test]
    fn empty_decoration_resolves_to_nothing() {
        assert!(
            CfDecorationPaint::resolve(CellDecoration::default(), &ColorIntern::new()).is_none()
        );
    }

    fn cell_rect() -> PixelRect {
        PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: 100,
            height: 20,
        }
    }

    #[test]
    fn a_hidden_category_hides_the_value() {
        let mut spec = bar("#3366cc", 1.0, 0.0, false);
        spec.show_value = false;
        let deco = CellDecoration {
            data_bar: Some(spec),
            ..CellDecoration::default()
        };
        let paint = CfDecorationPaint::resolve(deco, &ColorIntern::new()).expect("a bar");
        assert!(paint.hides_value());
    }

    #[test]
    fn a_shown_icon_reserves_a_left_band_and_a_hidden_one_does_not() {
        let shown = CellDecoration {
            icon: Some(icon(IconGlyph::ArrowUp, None)),
            ..CellDecoration::default()
        };
        let paint = CfDecorationPaint::resolve(shown, &ColorIntern::new()).expect("an icon");
        let reserved = paint.reserved_left(cell_rect());
        assert!(reserved > 0 && reserved < cell_rect().width);
        assert!(!paint.hides_value());

        let mut hidden_icon = icon(IconGlyph::ArrowUp, None);
        hidden_icon.show_value = false;
        let hidden = CellDecoration {
            icon: Some(hidden_icon),
            ..CellDecoration::default()
        };
        let paint = CfDecorationPaint::resolve(hidden, &ColorIntern::new()).expect("an icon");
        assert_eq!(paint.reserved_left(cell_rect()), 0);
    }

    #[test]
    fn simultaneous_categories_all_survive_resolution() {
        let deco = CellDecoration {
            icon: Some(icon(IconGlyph::Circle, None)),
            data_bar: Some(bar("#3366cc", 0.5, 0.0, false)),
            rating: Some(RatingSpec {
                glyph: IconGlyph::Heart,
                color: None,
                count: 2,
                max: 5,
                show_value: true,
            }),
        };
        match CfDecorationPaint::resolve(deco, &ColorIntern::new()) {
            Some(paint) => {
                assert!(paint.icon.is_some());
                assert!(paint.data_bar.is_some());
                assert!(paint.rating.is_some());
            }
            other => panic!("expected all three, got {other:?}"),
        }
    }
}
