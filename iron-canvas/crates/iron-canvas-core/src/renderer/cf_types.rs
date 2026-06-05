//! Paint-ready conditional formatting decoration types.
//!
//! These mirror IronCalc's `CfIcon`, `CfDataBar`, `CfRating` but are
//! pre-processed for the paint loop: hex strings parsed to `[u8; 3]`,
//! and fraction values clamped to renderable ranges.

use ironcalc_base::cf_types::{CfDataBar, CfIcon, CfRating, ExtendedStyle, Icon};
use serde::{Deserialize, Serialize};

/// Paint-ready icon decoration for a cell. The `icon` field uses IronCalc's
/// `Icon` enum — the painter maps its variant to a glyph/emoji/SVG path at
/// draw time so we don't carry font-dependent codepoints in paint data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfIconPaint {
    pub icon: Icon,
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
    /// Resolve the CF decoration from an IronCalc `ExtendedStyle`. Returns
    /// `None` when no CF decoration applies (plain cell).
    pub fn from_extended_style(extended: &ExtendedStyle) -> Option<Self> {
        if let Some(icon) = &extended.icon {
            return Some(CfDecorationPaint::Icon(CfIconPaint::from_cf_icon(icon)));
        }
        if let Some(bar) = &extended.data_bar {
            return Some(CfDecorationPaint::DataBar(CfDataBarPaint::from_cf_data_bar(
                bar,
            )));
        }
        if let Some(rating) = &extended.rating {
            return Some(CfDecorationPaint::from_cf_rating(rating));
        }
        None
    }
}

impl CfIconPaint {
    pub fn from_cf_icon(cf: &CfIcon) -> Self {
        Self {
            icon: cf.icon.clone(),
            color_rgb: parse_hex_color(&cf.color).unwrap_or([0, 0, 0]),
        }
    }
}

impl CfDataBarPaint {
    pub fn from_cf_data_bar(cf: &CfDataBar) -> Self {
        Self {
            fill_color_rgb: parse_hex_color(&cf.positive_color).unwrap_or([0, 0, 0]),
            fill_fraction: cf.value.clamp(0.0, 1.0),
        }
    }
}

impl CfDecorationPaint {
    pub fn from_cf_rating(cf: &CfRating) -> Self {
        CfDecorationPaint::Rating {
            stars: cf.max as u8,
            filled: cf.count as u8,
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
        let cf = CfDataBar {
            positive_color: "#3366CC".to_string(),
            negative_color: "#CC0000".to_string(),
            is_gradient: false,
            value: 1.5, // out of range — must clamp to 1.0
            axis_position: 0.0,
            show_value: true,
        };
        let paint = CfDataBarPaint::from_cf_data_bar(&cf);
        assert_eq!(paint.fill_color_rgb, [0x33, 0x66, 0xCC]);
        assert_eq!(paint.fill_fraction, 1.0);
    }

    #[test]
    fn rating_maps_max_to_stars_and_count_to_filled() {
        let cf = CfRating {
            icon: Icon::Circle,
            count: 3,
            max: 5,
            color: "#000000".to_string(),
            show_value: false,
        };
        match CfDecorationPaint::from_cf_rating(&cf) {
            CfDecorationPaint::Rating { stars, filled } => {
                assert_eq!(stars, 5);
                assert_eq!(filled, 3);
            }
            other => panic!("expected Rating, got {other:?}"),
        }
    }
}
