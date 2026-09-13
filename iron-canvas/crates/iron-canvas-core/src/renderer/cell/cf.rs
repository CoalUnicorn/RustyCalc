//! Resolved conditional-formatting decoration paints.
//!
//! Cell tier, next to the other `*Paint` records ([`super::paint::CellPaint`],
//! [`super::borders::BorderPaint`], [`super::text::TextPaint`]). The
//! resolve/paint split is the module's contract:
//! `CfDecorationPaint::resolve` is the only step in this module that may allocate. It
//! parses the color, clamps the fraction, and interns the data-bar CSS color
//! once per unique RGB triple. `CfDecorationPaint::paint` passes borrowed
//! colors and stack vertices to the backend. Backends may allocate.
//!
//! These are renderer paint records, not wire types: the recorder, SVG, and
//! PDF surfaces serialize `Painter` primitives (`rect_fill` / `fill_path`),
//! so no backend carries a CF-specific method.

use std::rc::Rc;

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::painter::{PaintColor, Painter};
use crate::renderer::cache::ColorIntern;
use crate::renderer::cache::color::data_bar_rgb;
use crate::style::CellDecoration;

/// Resolved icon decoration for a cell. The `icon` field is a String
/// placeholder (IconSpec); icon glyphs await a font/glyph system, so the
/// `Icon` arm of `CfDecorationPaint::paint` is a no-op for now and no
/// painted pixel depends on a richer icon enum yet.
#[derive(Debug, Clone, PartialEq)]
pub struct CfIconPaint {
    pub icon: String, // IconSpec placeholder — not yet painted
    pub color_rgb: [u8; 3],
}

/// Resolved data bar decoration for a cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CfDataBarPaint {
    /// Interned `#rrggbb` CSS color from the renderer's [`ColorIntern`]:
    /// `Rc::clone` after the first sighting of each rgb triple, so the paint
    /// step never formats a color per cell.
    pub fill_css: Rc<str>,
    /// Proportion of the bar to fill, clamped to [0.0, 1.0].
    pub fill_fraction: f64,
}

/// Resolved CF decoration enum. One per cell — at most one decoration
/// applies (icon, data bar, or rating), following IronCalc's priority model
/// where the last-matching rule in the evaluated result order wins.
#[derive(Debug, Clone, PartialEq)]
pub enum CfDecorationPaint {
    Icon(CfIconPaint),
    DataBar(CfDataBarPaint),
    Rating { stars: u8, filled: u8 },
}

impl CfDecorationPaint {
    /// Resolve a core `CellDecoration` into a renderer-ready paint.
    ///
    /// Takes the decoration by value: the caller owns it (the bulk fetch
    /// hands it over through `Fetched::take_value`), so the icon name moves
    /// instead of cloning. The data-bar color is interned here — one
    /// `format!` per unique rgb triple per renderer lifetime, not per
    /// decorated cell per frame.
    pub(super) fn resolve(deco: CellDecoration, intern: &ColorIntern) -> Self {
        match deco {
            CellDecoration::Icon(name) => CfDecorationPaint::Icon(CfIconPaint {
                icon: name,
                color_rgb: [0, 0, 0], // unused until icon glyphs are painted
            }),
            CellDecoration::DataBar(spec) => CfDecorationPaint::DataBar(CfDataBarPaint {
                fill_css: intern.get_rgb(data_bar_rgb(&spec)),
                fill_fraction: spec.fraction.clamp(0.0, 1.0),
            }),
            // RatingSpec fields are u32; CfDecorationPaint::Rating is u8.
            CellDecoration::Rating(spec) => CfDecorationPaint::Rating {
                stars: spec.stars as u8,
                filled: spec.filled as u8,
            },
        }
    }

    /// Paint this decoration over the already-filled cell `rect`, expressed
    /// purely in `Painter` primitives. The renderer owns CF geometry so the
    /// backend stays primitive-only: data bars become a `rect_fill` scaled by
    /// the fill fraction; ratings become `fill_path` star polygons. The icon
    /// variant is a placeholder (no glyph system yet) and paints nothing.
    pub(super) fn paint<P: Painter + ?Sized>(&self, painter: &P, rect: PixelRect) {
        match self {
            CfDecorationPaint::DataBar(bar) => {
                // Inset so the bar clears the grid/border strokes, then scale
                // its width by the clamped fraction.
                let inner = rect.inset(CF_INSET, CF_INSET);
                let bar_w = (f64::from(inner.width) * bar.fill_fraction).round() as i32;
                if inner.width <= 0 || inner.height <= 0 || bar_w <= 0 {
                    return;
                }
                let bar_rect = PixelRect {
                    top_left: inner.top_left,
                    width: bar_w,
                    height: inner.height,
                };
                painter.rect_fill(bar_rect, PaintColor::Borrowed(&bar.fill_css));
            }
            CfDecorationPaint::Rating { stars, filled } => {
                paint_rating(painter, rect, *stars, *filled);
            }
            // Icon glyphs need a font/glyph system that does not exist yet.
            CfDecorationPaint::Icon(_) => {}
        }
    }
}

/// Pixel inset applied to a cell rect before painting a CF decoration, so the
/// decoration never overlaps grid lines or explicit borders.
const CF_INSET: i32 = 2;

/// Gold for filled rating stars; light gray for empty ones. Hardcoded rather
/// than themed — Excel ratings are a fixed gold star regardless of theme.
const RATING_FILLED: &str = "#f0a30a";
const RATING_EMPTY: &str = "#d0d0d0";

/// Paint a left-to-right row of `stars` star glyphs, the first `filled` of
/// them solid gold and the rest light gray. Each star fits an equal-width
/// horizontal slot, sized to the smaller of the slot width and row height.
fn paint_rating<P: Painter + ?Sized>(painter: &P, rect: PixelRect, stars: u8, filled: u8) {
    let inner = rect.inset(CF_INSET, CF_INSET);
    if stars == 0 || inner.width <= 0 || inner.height <= 0 {
        return;
    }
    let slot_w = inner.width / i32::from(stars);
    let outer_r = f64::from(slot_w.min(inner.height)) / 2.0 - 1.0;
    if slot_w <= 0 || outer_r <= 0.0 {
        return;
    }
    let cy = inner.top() + inner.height / 2;
    for i in 0..i32::from(stars) {
        let cx = inner.left() + slot_w * i + slot_w / 2;
        let pts = star_points(Point { x: cx, y: cy }, outer_r);
        let color = if (i as u8) < filled {
            RATING_FILLED
        } else {
            RATING_EMPTY
        };
        painter.fill_path(&pts, PaintColor::Static(color));
    }
}

/// The ten vertices of a five-pointed star centered at `center`, starting at
/// the top tip and alternating outer/inner radius every 36°. The inner radius
/// is the canonical `0.382 × outer` that gives a regular pentagram. The
/// length is the constant `TIPS * 2`, so the vertices live on the stack.
fn star_points(center: Point, outer_r: f64) -> [Point; 10] {
    const TIPS: usize = 5;
    let inner_r = outer_r * 0.382;
    std::array::from_fn(|k| {
        let r = if k % 2 == 0 { outer_r } else { inner_r };
        let angle = -std::f64::consts::FRAC_PI_2 + (k as f64) * std::f64::consts::PI / TIPS as f64;
        Point {
            x: center.x + (r * angle.cos()).round() as i32,
            y: center.y + (r * angle.sin()).round() as i32,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{DataBarSpec, RatingSpec};

    #[test]
    fn data_bar_clamps_fraction_and_normalizes_color() {
        let intern = ColorIntern::new();
        let spec = DataBarSpec {
            color: "#3366CC".to_string(),
            fraction: 1.5, // out of range — must clamp to 1.0
        };
        let paint = CfDecorationPaint::resolve(CellDecoration::DataBar(spec), &intern);
        match paint {
            CfDecorationPaint::DataBar(p) => {
                assert_eq!(&*p.fill_css, "#3366cc");
                assert_eq!(p.fill_fraction, 1.0);
            }
            other => panic!("expected DataBar, got {other:?}"),
        }
    }

    #[test]
    fn data_bar_reuses_one_interned_color_per_rgb() {
        let intern = ColorIntern::new();
        let spec = DataBarSpec {
            color: "#3366CC".to_string(),
            fraction: 0.5,
        };
        let first = CfDecorationPaint::resolve(CellDecoration::DataBar(spec.clone()), &intern);
        let second = CfDecorationPaint::resolve(CellDecoration::DataBar(spec), &intern);
        match (first, second) {
            (CfDecorationPaint::DataBar(a), CfDecorationPaint::DataBar(b)) => assert!(
                Rc::ptr_eq(&a.fill_css, &b.fill_css),
                "repeat colors must reuse the interned string, not re-format it"
            ),
            other => panic!("expected two DataBars, got {other:?}"),
        }
    }

    #[test]
    fn rating_maps_stars_and_filled() {
        let spec = RatingSpec {
            stars: 5,
            filled: 3,
        };
        let paint = CfDecorationPaint::resolve(CellDecoration::Rating(spec), &ColorIntern::new());
        match paint {
            CfDecorationPaint::Rating { stars, filled } => {
                assert_eq!(stars, 5);
                assert_eq!(filled, 3);
            }
            other => panic!("expected Rating, got {other:?}"),
        }
    }

    #[test]
    fn icon_moves_the_name_and_carries_zeroed_color() {
        let name = "ArrowUp".to_string();
        let name_ptr = name.as_ptr();
        let paint = CfDecorationPaint::resolve(CellDecoration::Icon(name), &ColorIntern::new());
        match paint {
            CfDecorationPaint::Icon(p) => {
                assert_eq!(p.icon, "ArrowUp");
                assert_eq!(p.icon.as_ptr(), name_ptr, "resolution must move the name");
                assert_eq!(p.color_rgb, [0, 0, 0]);
            }
            other => panic!("expected Icon, got {other:?}"),
        }
    }

    #[test]
    fn star_has_ten_vertices_with_top_tip_first() {
        let pts = star_points(Point { x: 50, y: 50 }, 10.0);
        assert_eq!(pts.len(), 10);
        // First vertex is the outer tip pointing straight up: same column,
        // a full outer radius above center.
        assert_eq!(pts[0].x, 50);
        assert_eq!(pts[0].y, 40);
    }
}
