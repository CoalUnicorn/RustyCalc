use crate::model::style_types::is_valid_hex_color;

#[test]
fn test_hex_color_validation() {
    // Testing the unified validation function from style_types
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
