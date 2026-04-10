//! Domain types for spreadsheet styling operations.
//!
//! This module provides strongly-typed alternatives to the bare strings used
//! in IronCalc's `update_range_style()` API.

/// A validated style property path for IronCalc's `update_range_style()` API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StylePath(&'static str);

impl StylePath {
    /// Background fill color: `"fill.fg_color"`
    /// - Empty string clears the background (transparent)
    /// - Hex string like `"#FF0000"` sets the color
    /// - IronCalc automatically sets pattern_type = "solid"
    pub const BACKGROUND_COLOR: Self = Self("fill.fg_color");

    /// Font text color: `"font.color"`
    /// - Empty string clears color (uses theme default)
    /// - Hex string like `"#000000"` sets the color
    pub const TEXT_COLOR: Self = Self("font.color");

    /// Font weight (bold): `"font.b"`
    /// - `"true"` makes text bold
    /// - `"false"` removes bold
    pub const FONT_BOLD: Self = Self("font.b");

    /// Font style (italic): `"font.i"`
    /// - `"true"` makes text italic
    /// - `"false"` removes italic
    pub const FONT_ITALIC: Self = Self("font.i");

    /// Text decoration (underline): `"font.u"`
    /// - `"true"` adds underline
    /// - `"false"` removes underline
    pub const FONT_UNDERLINE: Self = Self("font.u");

    /// Text decoration (strikethrough): `"font.strike"`
    /// - `"true"` adds strikethrough
    /// - `"false"` removes strikethrough
    pub const FONT_STRIKETHROUGH: Self = Self("font.strike");

    /// Font size delta: `"font.size_delta"`
    /// - Integer string like `"2"` or `"-3"` adjusts from base size
    /// - Used for relative size changes in formatting operations
    pub const FONT_SIZE_DELTA: Self = Self("font.size_delta");

    /// Returns the IronCalc-compatible string path.
    pub fn as_str(&self) -> &'static str {
        self.0
    }
}

impl AsRef<str> for StylePath {
    fn as_ref(&self) -> &str {
        self.0
    }
}

impl std::fmt::Display for StylePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A validated CSS hex color value.
///
/// Ensures color strings are either empty (transparent) or valid hex format.
/// Prevents runtime errors from malformed color values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexColor(String);

#[derive(Debug, thiserror::Error)]
pub enum ColorError {
    #[error("Invalid hex color format: '{color}' (expected #RRGGBB)")]
    InvalidFormat { color: String },
}

impl HexColor {
    /// Creates a hex color, validating the format.
    ///
    /// # Examples
    /// ```rust
    /// let red = HexColor::new("#FF0000")?;
    /// let transparent = HexColor::transparent();
    /// ```
    pub fn new(hex: impl Into<String>) -> Result<Self, ColorError> {
        let hex = hex.into();

        // Empty string is valid (means transparent/clear)
        if hex.is_empty() {
            return Ok(Self(hex));
        }

        if !is_valid_hex_color(&hex) {
            return Err(ColorError::InvalidFormat { color: hex });
        }

        // Normalize to 6-digit format (#RRGGBB)
        let normalized = normalize_hex_color(&hex);
        Ok(Self(normalized))
    }

    /// Creates a transparent color (empty string).
    pub fn transparent() -> Self {
        Self(String::new())
    }

    /// Returns the hex color string for IronCalc APIs.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_transparent(&self) -> bool {
        self.0.is_empty()
    }

    /// Converts an `Option<String>` to a `HexColor` at a system boundary.
    ///
    /// `None` and invalid hex strings both map to `transparent()`.
    /// Use this in UI callbacks where the color picker may produce unvalidated input.
    pub fn from_opt(opt: Option<String>) -> Self {
        match opt {
            None => Self::transparent(),
            Some(s) if s.is_empty() => Self::transparent(),
            Some(s) => Self::new(s).unwrap_or_else(|_| Self::transparent()),
        }
    }
}

impl AsRef<str> for HexColor {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HexColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_transparent() {
            write!(f, "transparent")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// Unified hex color validation used by both UI components and domain types.
///
/// Accepts both 3-digit (`#RGB`) and 6-digit (`#RRGGBB`) hex colors.
/// This matches the validation logic from `color_picker.rs` to ensure consistency.
pub fn is_valid_hex_color(hex: &str) -> bool {
    if !hex.starts_with('#') {
        return false;
    }
    let digits = &hex[1..];
    matches!(digits.len(), 3 | 6) && digits.chars().all(|c| c.is_ascii_hexdigit())
}

/// Convert a 3-digit hex color to 6-digit format.
///
/// Examples: `#RGB` -> `#RRGGBB`, `#f0a` -> `#ff00aa`
pub fn normalize_hex_color(hex: &str) -> String {
    if hex.len() == 4 && hex.starts_with('#') {
        // #RGB -> #RRGGBB
        let r = &hex[1..2];
        let g = &hex[2..3];
        let b = &hex[3..4];
        format!("#{r}{r}{g}{g}{b}{b}")
    } else {
        hex.to_string()
    }
}

/// A boolean toggle value for IronCalc style operations.
///
/// IronCalc expects `"true"` or `"false"` strings for boolean properties.
/// This type ensures we never pass invalid boolean representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanValue {
    True,
    False,
}

impl BooleanValue {
    pub fn from_bool(value: bool) -> Self {
        if value {
            Self::True
        } else {
            Self::False
        }
    }

    /// Returns the IronCalc-compatible string ("true" or "false").
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::True => "true",
            Self::False => "false",
        }
    }

    pub fn as_bool(&self) -> bool {
        matches!(self, Self::True)
    }

    /// Returns the opposite boolean value.
    pub fn toggle(&self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
        }
    }
}

impl From<bool> for BooleanValue {
    fn from(value: bool) -> Self {
        Self::from_bool(value)
    }
}

impl From<BooleanValue> for bool {
    fn from(value: BooleanValue) -> Self {
        value.as_bool()
    }
}

impl AsRef<str> for BooleanValue {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_path_constants() {
        assert_eq!(StylePath::BACKGROUND_COLOR.as_str(), "fill.fg_color");
        assert_eq!(StylePath::FONT_BOLD.as_str(), "font.b");
        assert_eq!(StylePath::TEXT_COLOR.as_str(), "font.color");
    }

    #[test]
    fn hex_color_validation() {
        // Valid colors
        assert!(HexColor::new("#FF0000").is_ok());
        assert!(HexColor::new("#000000").is_ok());
        assert!(HexColor::new("#ABC").is_ok()); // 3-digit
        assert!(HexColor::new("").is_ok()); // Transparent

        // Invalid colors
        assert!(HexColor::new("FF0000").is_err()); // No #
        assert!(HexColor::new("#FF00").is_err()); // Wrong length
        assert!(HexColor::new("#GG0000").is_err()); // Invalid hex
    }

    #[allow(clippy::unwrap_used)]
    #[test]
    fn hex_color_normalization() {
        // 3-digit colors get normalized to 6-digit
        assert_eq!(HexColor::new("#ABC").unwrap().as_str(), "#AABBCC");
        assert_eq!(HexColor::new("#f0a").unwrap().as_str(), "#ff00aa");

        // 6-digit colors stay unchanged
        assert_eq!(HexColor::new("#FF0000").unwrap().as_str(), "#FF0000");

        // Transparent stays empty
        assert_eq!(HexColor::transparent().as_str(), "");
    }

    #[test]
    fn unified_validation_matches_color_picker() {
        // Test cases that should match color_picker.rs validation
        assert!(is_valid_hex_color("#000"));
        assert!(is_valid_hex_color("#000000"));
        assert!(is_valid_hex_color("#ABC"));
        assert!(is_valid_hex_color("#abcdef"));
        assert!(is_valid_hex_color("#123456"));

        assert!(!is_valid_hex_color("000"));
        assert!(!is_valid_hex_color("#"));
        assert!(!is_valid_hex_color("#00"));
        assert!(!is_valid_hex_color("#0000"));
        assert!(!is_valid_hex_color("#00000"));
        assert!(!is_valid_hex_color("#0000000"));
        assert!(!is_valid_hex_color("#xyz"));
        assert!(!is_valid_hex_color("#gggggg"));
    }

    #[test]
    fn boolean_value_conversion() {
        assert_eq!(BooleanValue::from_bool(true).as_str(), "true");
        assert_eq!(BooleanValue::from_bool(false).as_str(), "false");
        assert_eq!(BooleanValue::True.toggle(), BooleanValue::False);
    }
}
