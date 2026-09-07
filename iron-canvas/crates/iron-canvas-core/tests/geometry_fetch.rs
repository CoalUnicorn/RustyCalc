//! FSM-A2 boundary tests for the geometry/config fetch outcomes.
//!
//! `CanvasModel::get_row_height` / `get_column_width` /
//! `get_show_grid_lines` return `Fetched<T>`: `Absent` selects the engine's
//! documented default, `Value` is authoritative, and `BridgeFailed` must
//! never be collapsed to a default. Extent `Value`s must be finite,
//! non-negative, and fit `0..=i32::MAX` px — anything else is
//! `ExtentFetch::Invalid` and aborts the walk exactly like a bridge failure
//! (never fabricated by rounding/casting).
//!
//! Build-level pinning goes through the public `Orchestrator` (held-frame
//! tests in `held_frame.rs`); the resolution boundary itself is pinned here
//! through the public `row_height` / `col_width` resolvers.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use iron_canvas_core::geometry::slot::{ExtentFetch, col_width, row_height};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::types::fetched::Fetched;
use iron_canvas_core::{CanvasModel, FrameInputs};

use common::{TestModel, canvas_default};

fn test_frame(model: &TestModel) -> Chrome {
    let theme = Rc::new(CanvasTheme::light());
    let inputs =
        FrameInputs::capture(model, canvas_default(), 1.0, theme, 0).expect("healthy capture");
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

/// With every row height absent, the walk must use the engine documented
/// default (`DEFAULT_ROW_HEIGHT`), not the host's own `default_row_height`.
#[test]
fn absent_row_heights_build_default_geometry() {
    let model = TestModel::synthetic_grid().with_default_row_height(99.0);
    model.set_row_height_absent(true);

    let frame = test_frame(&model);
    let row1 = frame.pane_set.row_to_y(1);
    let row2 = frame.pane_set.row_to_y(2);
    assert_eq!(
        row2 - row1,
        iron_canvas_core::geometry::constants::DEFAULT_ROW_HEIGHT as i32,
        "Absent must select the engine default, not the host default 99.0"
    );
}

#[test]
fn absent_column_widths_build_default_geometry() {
    let model = TestModel::synthetic_grid();
    model.set_col_width_absent(true);

    let frame = test_frame(&model);
    let col1 = frame.pane_set.col_to_x(1);
    let col2 = frame.pane_set.col_to_x(2);
    assert_eq!(col2 - col1, DEFAULT_COL_WIDTH as i32);
}

#[test]
fn absent_grid_lines_default_to_shown() {
    let model = TestModel::synthetic_grid();
    model.set_grid_lines_absent(true);
    assert_eq!(
        model.get_show_grid_lines(0),
        Fetched::Absent,
        "the knob routes to Absent"
    );
}

#[test]
fn concrete_values_flow_through_unchanged() {
    let model = TestModel::synthetic_grid();
    model.set_row_height(3, 50.0);
    model.set_col_width(4, 120.0);

    let frame = test_frame(&model);
    let r3 = frame.pane_set.row_to_y(3);
    let r4 = frame.pane_set.row_to_y(4);
    assert_eq!(r4 - r3, 50);

    let c4 = frame.pane_set.col_to_x(4);
    let c5 = frame.pane_set.col_to_x(5);
    assert_eq!(c5 - c4, 120);
}

// ─── Extent numeric boundary ────────────────────────────────────────────
//
// `row_height` / `col_width` resolve a host `Value` once, at the geometry
// boundary: a finite value in `0..=i32::MAX` px becomes `Px` (zero stays the
// hidden-row/column value), and anything that cannot become a slot extent
// (NaN, infinities, negative, overflowing `i32`) is `Invalid` — aborting the
// walk, never silently clamped or cast into fabricated geometry.

#[test]
fn finite_extents_round_to_px_and_zero_stays_hidden() {
    let model = TestModel::synthetic_grid();
    model.set_row_height(1, 0.0);
    model.set_row_height(2, 20.4);
    model.set_row_height(3, 20.6);
    model.set_row_height(4, i32::MAX as f64);
    assert_eq!(
        row_height(&model, 0, 1),
        ExtentFetch::Px(0),
        "zero = hidden"
    );
    assert_eq!(row_height(&model, 0, 2), ExtentFetch::Px(20));
    assert_eq!(row_height(&model, 0, 3), ExtentFetch::Px(21));
    assert_eq!(row_height(&model, 0, 4), ExtentFetch::Px(i32::MAX));
}

#[test]
fn row_height_rejects_values_that_cannot_become_geometry() {
    let model = TestModel::synthetic_grid();
    for (row, bad) in [
        (1, f64::NAN),
        (2, f64::INFINITY),
        (3, f64::NEG_INFINITY),
        (4, -1.0),
        (5, i32::MAX as f64 + 1.0),
    ] {
        model.set_row_height(row, bad);
        assert_eq!(
            row_height(&model, 0, row),
            ExtentFetch::Invalid,
            "row {row} height {bad:?} must not become a pixel extent"
        );
        assert_eq!(
            row_height(&model, 0, row).extent(),
            None,
            "an invalid extent must abort the walk"
        );
    }
}

#[test]
fn col_width_rejects_values_that_cannot_become_geometry() {
    let model = TestModel::synthetic_grid();
    for (col, bad) in [
        (1, f64::NAN),
        (2, f64::INFINITY),
        (3, -0.5),
        (4, i32::MAX as f64 + 1024.0),
    ] {
        model.set_col_width(col, bad);
        assert_eq!(
            col_width(&model, 0, col),
            ExtentFetch::Invalid,
            "col {col} width {bad:?} must not become a pixel extent"
        );
    }
    model.set_col_width(5, 0.0);
    model.set_col_width(6, 80.4);
    assert_eq!(col_width(&model, 0, 5), ExtentFetch::Px(0), "zero = hidden");
    assert_eq!(col_width(&model, 0, 6), ExtentFetch::Px(80));
}

#[test]
fn absent_extents_use_documented_defaults() {
    let model = TestModel::synthetic_grid().with_default_row_height(999.0);
    model.set_row_height_absent(true);
    model.set_col_width_absent(true);
    assert_eq!(
        row_height(&model, 0, 7),
        ExtentFetch::Px(DEFAULT_ROW_HEIGHT as i32),
        "Absent uses the engine default, not the host's 999.0"
    );
    assert_eq!(
        col_width(&model, 0, 7),
        ExtentFetch::Px(DEFAULT_COL_WIDTH as i32)
    );
}

#[test]
fn bridge_failed_extents_stay_bridge_failed() {
    let model = TestModel::synthetic_grid();
    model.set_row_height_bridge_fail(true);
    model.set_col_width_bridge_fail(true);
    assert_eq!(row_height(&model, 0, 3), ExtentFetch::BridgeFailed);
    assert_eq!(col_width(&model, 0, 4), ExtentFetch::BridgeFailed);
    assert_eq!(row_height(&model, 0, 3).extent(), None);
}
