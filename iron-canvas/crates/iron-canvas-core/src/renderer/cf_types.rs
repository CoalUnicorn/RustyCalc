//! Paint-ready conditional formatting decoration types.
//!
//! These are pre-processed for the paint loop: hex strings parsed to `[u8; 3]`
//! and fraction values clamped to renderable ranges. [`CfDecorationPaint::paint`]
//! resolves a decoration into `Painter` primitives (`rect_fill` / `fill_path`)
//! so no backend carries a CF-specific method — the recorder, SVG, and PDF
//! surfaces replay the same primitive ops.

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::painter::{PaintColor, Painter};
use crate::style::CellDecoration;
use serde::{Deserialize, Serialize};

/// Paint-ready icon decoration for a cell. The `icon` field is a String
/// placeholder (IconSpec); icon glyphs await a font/glyph system, so the
/// `Icon` arm of [`CfDecorationPaint::paint`] is a no-op for now and no
/// painted pixel depends on a richer icon enum yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfIconPaint {
    pub icon: String, // IconSpec placeholder — not yet painted
    pub color_rgb: [u8; 3],
}

/// Paint-ready data bar decoration for a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfDataBarPaint {
    pub fill_color_rgb: [u8; 3],
    /// Proportion of the bar to fill, clamped to [0.0, 1.0].
    pub fill_fraction: f64,
}

/// Paint-ready CF decoration enum. One per cell — at most one decoration
/// applies (icon, data bar, or rating), following IronCalc's priority model
/// where the last-matching rule in the evaluated result order wins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CfDecorationPaint {
    Icon(CfIconPaint),
    DataBar(CfDataBarPaint),
    Rating { stars: u8, filled: u8 },
}

impl CfDecorationPaint {
    /// Convert a core `CellDecoration` into a renderer-ready `CfDecorationPaint`.
    pub fn from_cell_decoration(deco: &CellDecoration) -> Self {
        match deco {
            CellDecoration::Icon(name) => CfDecorationPaint::Icon(CfIconPaint {
                icon: name.clone(),
                color_rgb: [0, 0, 0], // unused until icon glyphs are painted
            }),
            CellDecoration::DataBar(spec) => CfDecorationPaint::DataBar(CfDataBarPaint {
                fill_color_rgb: parse_hex_color(&spec.color).unwrap_or([0, 0, 0]),
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
    pub fn paint<P: Painter + ?Sized>(&self, painter: &P, rect: PixelRect) {
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
                let color = rgb_hex(bar.fill_color_rgb);
                painter.rect_fill(bar_rect, PaintColor::Borrowed(&color));
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
/// is the canonical `0.382 × outer` that gives a regular pentagram.
fn star_points(center: Point, outer_r: f64) -> Vec<Point> {
    const TIPS: usize = 5;
    let inner_r = outer_r * 0.382;
    (0..TIPS * 2)
        .map(|k| {
            let r = if k % 2 == 0 { outer_r } else { inner_r };
            let angle =
                -std::f64::consts::FRAC_PI_2 + (k as f64) * std::f64::consts::PI / TIPS as f64;
            Point {
                x: center.x + (r * angle.cos()).round() as i32,
                y: center.y + (r * angle.sin()).round() as i32,
            }
        })
        .collect()
}

/// Format an `[R, G, B]` triple as a `#rrggbb` CSS color string.
fn rgb_hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Parse a `#RRGGBB` hex string into `[R, G, B]`. Returns `None` for
/// invalid formats or non-hex characters.
///
/// `pub(crate)`: also used by `fingerprint.rs`'s `hash_decoration` to hash a
/// data bar's resolved color without constructing a `CfDecorationPaint`.
pub(crate) fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{DataBarSpec, RatingSpec};

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(parse_hex_color("#FF8000"), Some([255, 128, 0]));
        assert_eq!(parse_hex_color("00ff00"), Some([0, 255, 0]));
    }

    #[test]
    fn rejects_malformed_hex() {
        assert_eq!(parse_hex_color("#FFF"), None); // too short
        assert_eq!(parse_hex_color("#GGGGGG"), None); // non-hex
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn data_bar_clamps_fraction_and_uses_positive_color() {
        let spec = DataBarSpec {
            color: "#3366CC".to_string(),
            fraction: 1.5, // out of range — must clamp to 1.0
        };
        let paint = CfDecorationPaint::from_cell_decoration(&CellDecoration::DataBar(spec));
        match paint {
            CfDecorationPaint::DataBar(p) => {
                assert_eq!(p.fill_color_rgb, [0x33, 0x66, 0xCC]);
                assert_eq!(p.fill_fraction, 1.0);
            }
            other => panic!("expected DataBar, got {other:?}"),
        }
    }

    #[test]
    fn rating_maps_stars_and_filled() {
        let spec = RatingSpec {
            stars: 5,
            filled: 3,
        };
        let paint = CfDecorationPaint::from_cell_decoration(&CellDecoration::Rating(spec));
        match paint {
            CfDecorationPaint::Rating { stars, filled } => {
                assert_eq!(stars, 5);
                assert_eq!(filled, 3);
            }
            other => panic!("expected Rating, got {other:?}"),
        }
    }

    #[test]
    fn icon_carries_name_and_zeroed_color() {
        let paint =
            CfDecorationPaint::from_cell_decoration(&CellDecoration::Icon("ArrowUp".to_string()));
        match paint {
            CfDecorationPaint::Icon(p) => {
                assert_eq!(p.icon, "ArrowUp");
                assert_eq!(p.color_rgb, [0, 0, 0]);
            }
            other => panic!("expected Icon, got {other:?}"),
        }
    }

    #[test]
    fn rgb_hex_lowercases_and_zero_pads() {
        assert_eq!(rgb_hex([0x33, 0x66, 0xCC]), "#3366cc");
        assert_eq!(rgb_hex([0, 8, 255]), "#0008ff");
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
