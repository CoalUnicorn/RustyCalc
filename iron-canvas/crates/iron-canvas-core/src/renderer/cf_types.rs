//! Paint-ready conditional formatting decoration types.
//!
//! These are pre-processed for the paint loop: hex strings parsed to `[u8; 3]`
//! and fraction values clamped to renderable ranges.

use crate::style::CellDecoration;
use serde::{Deserialize, Serialize};

/// Paint-ready icon decoration for a cell. The `icon` field is a String
/// placeholder (IconSpec) — `paint_cf_decoration` is a no-op on all
/// backends today, so no painted pixel depends on a richer icon enum yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfIconPaint {
    pub icon: String, // IconSpec placeholder — paint_cf_decoration is a no-op
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
                color_rgb: [0, 0, 0], // unused: paint_cf_decoration is a no-op
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
}

/// Parse a `#RRGGBB` hex string into `[R, G, B]`. Returns `None` for
/// invalid formats or non-hex characters.
fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
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
}
