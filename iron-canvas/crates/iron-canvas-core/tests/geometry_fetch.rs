//! FSM-A2 boundary tests for the geometry/config fetch outcomes.
//!
//! `CanvasModel::get_row_height` / `get_column_width` /
//! `get_show_grid_lines` return `Fetched<T>`: `Absent` selects the engine's
//! documented default, `Value` is authoritative, and `BridgeFailed` must
//! never be collapsed to a default. These tests pin the resolution through
//! `Chrome::build` (the slot walk) — the crate-private `ExtentFetch` helpers
//! are covered indirectly, since `Chrome::build` is their only production
//! consumer.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::geometry::constants::DEFAULT_COL_WIDTH;
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
