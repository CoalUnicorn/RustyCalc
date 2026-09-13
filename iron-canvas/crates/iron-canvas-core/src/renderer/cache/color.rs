//! One decoder for the model's conditional-formatting color strings.
//!
//! A data bar carries a CSS color string. Two tiers consume the same rgb
//! triple from it: the cell tier paints it through
//! [`ColorIntern`](super::ColorIntern), and the cache tier hashes it into a
//! cell digest ([`super::fingerprint`]). The decode and its malformed-input
//! fallback therefore live here, not in either consumer.

use crate::style::DataBarSpec;

/// The data bar's fill color as `[R, G, B]`. A malformed model color falls
/// back to black, the same value the painter falls back to.
pub(crate) fn data_bar_rgb(spec: &DataBarSpec) -> [u8; 3] {
    parse_hex_color(&spec.color).unwrap_or([0, 0, 0])
}

/// Parse a `#RRGGBB` hex string into `[R, G, B]`. Returns `None` for
/// invalid formats or non-hex characters.
fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
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

    fn rgb(color: &str) -> [u8; 3] {
        data_bar_rgb(&DataBarSpec {
            color: color.to_string(),
            fraction: 0.5,
        })
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(rgb("#FF8000"), [255, 128, 0]);
        assert_eq!(rgb("00ff00"), [0, 255, 0]);
    }

    #[test]
    fn rejects_malformed_hex() {
        assert_eq!(rgb("#FFF"), [0, 0, 0]); // too short
        assert_eq!(rgb("#GGGGGG"), [0, 0, 0]); // non-hex
        assert_eq!(rgb(""), [0, 0, 0]);
    }

    #[test]
    fn rejects_non_ascii_hex_without_panicking() {
        for color in ["#aé000", "#00aé0", "#0000é", "#中文"] {
            assert_eq!(rgb(color), [0, 0, 0], "{color}");
        }
    }

    #[test]
    fn rejects_signed_hex_components() {
        for color in ["#+10000", "#00+100", "#0000+1"] {
            assert_eq!(rgb(color), [0, 0, 0], "{color}");
        }
    }
}
