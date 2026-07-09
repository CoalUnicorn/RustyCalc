use crate::model::style_types::*;

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
