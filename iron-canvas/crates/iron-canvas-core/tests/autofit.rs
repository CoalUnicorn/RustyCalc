//! Pure auto-fit measurement: `fit_width` / `fit_height` over an explicit
//! used-range span. A fixed-width `TextMetrics` stub keeps the assertion
//! arithmetic exact.

#![allow(clippy::unwrap_used)]

mod common;

use iron_canvas_core::autofit::{FIT_PADDING, fit_height, fit_width, font_css};
use iron_canvas_core::painter::TextMetrics;

use common::TestModel;

/// Returns `chars * 10.0` regardless of font — the production painter reads
/// `font_css`, but the test pins arithmetic, not font resolution.
struct CharWidth;
impl TextMetrics for CharWidth {
    fn measure_text_width(&self, text: &str, _font_css: &str) -> f64 {
        text.chars().count() as f64 * 10.0
    }
}

#[test]
fn fit_width_returns_widest_value_plus_padding() {
    // col 2, rows 1..=3: "", "hello" (5), "ab" (2). Widest = 5 * 10 = 50.
    let model = TestModel::synthetic_grid();
    model.set_cell(2, 2, "hello");
    model.set_cell(3, 2, "ab");
    let metrics = CharWidth;
    assert_eq!(
        fit_width(&model, &metrics, 2, 1, 3),
        Some(50.0 + FIT_PADDING)
    );
}

#[test]
fn fit_width_caps_scan_at_fit_scan_cap_rows() {
    use iron_canvas_core::autofit::FIT_SCAN_CAP;
    // Narrow value just inside the cap; wide value one row past it. With a
    // top-down cap the row past FIT_SCAN_CAP is never measured, so the wide
    // value can't influence the fitted width.
    let model = TestModel::synthetic_grid();
    model.set_cell(FIT_SCAN_CAP, 2, "ab"); // 2 chars -> 20, last scanned row
    model.set_cell(FIT_SCAN_CAP + 1, 2, "wwwwwwwwww"); // beyond the cap
    let metrics = CharWidth;
    assert_eq!(
        fit_width(&model, &metrics, 2, 1, FIT_SCAN_CAP + 1),
        Some(20.0 + FIT_PADDING)
    );
}

#[test]
fn fit_width_returns_none_for_empty_column() {
    let model = TestModel::synthetic_grid();
    let metrics = CharWidth;
    assert_eq!(fit_width(&model, &metrics, 2, 1, 3), None);
}

#[test]
fn fit_height_returns_none_for_empty_row() {
    let model = TestModel::synthetic_grid();
    let metrics = CharWidth;
    assert_eq!(fit_height(&model, &metrics, 2, 1, 3), None);
}

#[test]
fn fit_height_returns_some_when_row_has_content() {
    // Default Style has sz = 13; expected result is 13.0 + FIT_PADDING.
    let model = TestModel::synthetic_grid();
    model.set_cell(2, 1, "x");
    let metrics = CharWidth;
    assert_eq!(
        fit_height(&model, &metrics, 2, 1, 3),
        Some(13.0 + FIT_PADDING)
    );
}

#[test]
fn font_css_quotes_multi_word_family() {
    // "Times New Roman" contains spaces, so escape_font_family must wrap it
    // in double quotes. This asserts font_css produces the same quoted form
    // that the renderer's FontIntern / cache::font::build would produce.
    use ironcalc_base::types::Style;
    let mut style = Style::default();
    style.font.name = "Times New Roman".to_owned();
    assert_eq!(font_css(&style), "13px \"Times New Roman\"");
}

#[test]
fn font_css_empty_family_falls_back_to_calibri() {
    use ironcalc_base::types::Style;
    let style = Style::default(); // font.name is "" by default
    // sz=13, not bold, not italic → "13px Calibri"
    assert_eq!(font_css(&style), "13px Calibri");
}
