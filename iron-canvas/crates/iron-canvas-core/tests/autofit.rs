//! Pure auto-fit measurement: `fit_width` / `fit_height` over an explicit
//! used-range span. A fixed-width `TextMetrics` stub keeps the assertion
//! arithmetic exact.

#![allow(clippy::unwrap_used)]

mod common;

use iron_canvas_core::autofit::{AutoFitError, FIT_PADDING, fit_height, fit_width, font_css};
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
fn failed_value_reads_are_errors_for_both_axes() {
    let model = TestModel::new();
    model.set_value_bridge_fail(true);
    assert_eq!(
        fit_width(&model, &CharWidth, 1, 1, 2),
        Err(AutoFitError::CellValue)
    );
    assert_eq!(
        fit_height(&model, &CharWidth, 1, 1, 2),
        Err(AutoFitError::CellValue)
    );
}

#[test]
fn late_style_failure_discards_partial_measurements() {
    let model = TestModel::new();
    model.set_cell(1, 1, "x");
    model.set_cell(2, 1, "long text");
    model.set_cell(1, 2, "long text");
    model.set_style_bridge_fail_at(Some((2, 1)));
    assert_eq!(
        fit_width(&model, &CharWidth, 1, 1, 2),
        Err(AutoFitError::CellStyle)
    );
    model.set_style_bridge_fail_at(Some((1, 2)));
    assert_eq!(
        fit_height(&model, &CharWidth, 1, 1, 2),
        Err(AutoFitError::CellStyle)
    );
    model.set_style_bridge_fail_at(None);
    assert!(
        fit_width(&model, &CharWidth, 1, 1, 2)
            .expect("recovered read")
            .is_some()
    );
    assert!(
        fit_height(&model, &CharWidth, 1, 1, 2)
            .expect("recovered read")
            .is_some()
    );
}

#[test]
fn inverted_ranges_do_not_scan_cells() {
    let model = TestModel::new();
    model.set_value_bridge_fail(true);
    assert_eq!(
        fit_width(&model, &CharWidth, 1, i32::MAX, i32::MIN),
        Ok(None)
    );
    assert_eq!(
        fit_height(&model, &CharWidth, 1, i32::MAX, i32::MIN),
        Ok(None)
    );
}

#[test]
fn unbound_orchestrator_reports_no_model() {
    use iron_canvas_core::Orchestrator;
    use iron_canvas_recorder::MemSurface;
    let orch = Orchestrator::new(MemSurface::new(), MemSurface::new());
    assert_eq!(orch.fit_column_width(1, 1, 1), Err(AutoFitError::NoModel));
    assert_eq!(orch.fit_row_height(1, 1, 1), Err(AutoFitError::NoModel));
}

#[test]
fn fit_width_reports_a_failed_sheet_read() {
    // A transient bridge failure is not "no content": the host must not
    // resize the column to a default on the strength of an unread sheet.
    let model = TestModel::synthetic_grid();
    model.set_cell(2, 2, "hello");
    model.set_capture_fail(Some(iron_canvas_core::FrameInputFailure::SelectedSheet));

    assert_eq!(
        fit_width(&model, &CharWidth, 2, 1, 3),
        Err(AutoFitError::SelectedSheet)
    );
}

#[test]
fn fit_height_rejects_invalid_column_extents() {
    let model = TestModel::synthetic_grid();
    model.set_cell(1, 1, "text");
    for width in [f64::NAN, f64::INFINITY, -1.0, i32::MAX as f64 + 1.0] {
        model.set_col_width(1, width);
        assert_eq!(
            fit_height(&model, &CharWidth, 1, 1, 1),
            Err(AutoFitError::ColumnExtent),
            "{width}"
        );
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
        Ok(Some(50.0 + FIT_PADDING))
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
        Ok(Some(20.0 + FIT_PADDING))
    );
}

#[test]
fn fit_width_returns_none_for_empty_column() {
    let model = TestModel::synthetic_grid();
    let metrics = CharWidth;
    assert_eq!(fit_width(&model, &metrics, 2, 1, 3), Ok(None));
}

#[test]
fn fit_height_returns_none_for_empty_row() {
    let model = TestModel::synthetic_grid();
    let metrics = CharWidth;
    assert_eq!(fit_height(&model, &metrics, 2, 1, 3), Ok(None));
}

#[test]
fn fit_height_returns_some_when_row_has_content() {
    // CellStyle::default() has size=11.0; expected result is 11.0 + FIT_PADDING.
    let model = TestModel::synthetic_grid();
    model.set_cell(2, 1, "x");
    let metrics = CharWidth;
    assert_eq!(
        fit_height(&model, &metrics, 2, 1, 3),
        Ok(Some(11.0 + FIT_PADDING))
    );
}

#[test]
fn font_css_quotes_multi_word_family() {
    // "Times New Roman" contains spaces, so escape_font_family must wrap it
    // in double quotes. This asserts font_css produces the same quoted form
    // that the renderer's FontIntern / cache::font::build would produce.
    use iron_canvas_core::CellStyle;
    let mut style = CellStyle::default();
    style.font.name = "Times New Roman".to_owned();
    assert_eq!(font_css(&style), "11px \"Times New Roman\"");
}

#[test]
fn font_css_empty_family_falls_back_to_calibri() {
    use iron_canvas_core::CellStyle;
    let style = CellStyle::default(); // font.name is "" by default
    // size=11.0, not bold, not italic -> "11px Calibri"
    assert_eq!(font_css(&style), "11px Calibri");
}
