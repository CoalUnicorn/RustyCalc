//! Stage 3 Task 1 — `FrameInputs::capture`: the fallible, once-per-attempt
//! scalar snapshot. Two concerns:
//!
//! - the counting model proves `capture` reads each scalar accessor exactly
//!   once (the fixed 7-step order the plan requires never double-reads or
//!   skips a step);
//! - the table tests prove every `FrameInputFailure` variant is reachable,
//!   holds the attempt (`Err`, not a substituted default), and that a
//!   divergent selected-sheet/view pair is caught as `SheetMismatch` rather
//!   than silently building a mixed frame.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    CanvasModel, CanvasView, CellContentQuery, CellKind, CellStyle, Fetched, FrameInputFailure,
    FrameInputs,
};

use common::{TestModel, canvas_default};

/// Wraps a `TestModel`, counting calls to each scalar accessor
/// `FrameInputs::capture` is specified to read. Composition, not
/// inheritance-by-trait-default: every method under test increments its own
/// counter then forwards to `inner`; everything else forwards untouched.
#[derive(Default)]
struct CountingModel {
    inner: TestModel,
    sheet_calls: Cell<u32>,
    view_calls: Cell<u32>,
    frozen_rows_calls: Cell<u32>,
    frozen_cols_calls: Cell<u32>,
    show_row_headers_calls: Cell<u32>,
    show_col_headers_calls: Cell<u32>,
    show_selection_calls: Cell<u32>,
}

impl CellContentQuery for CountingModel {
    fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellStyle> {
        self.inner.get_cell_style(sheet, row, column)
    }
    fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> Fetched<CellKind> {
        self.inner.get_cell_type(sheet, row, column)
    }
    fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> Fetched<String> {
        self.inner.get_formatted_cell_value(sheet, row, column)
    }
}

impl CanvasModel for CountingModel {
    fn get_selected_sheet(&self) -> Option<u32> {
        self.sheet_calls.set(self.sheet_calls.get() + 1);
        self.inner.get_selected_sheet()
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        self.view_calls.set(self.view_calls.get() + 1);
        self.inner.get_selected_view()
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.frozen_rows_calls.set(self.frozen_rows_calls.get() + 1);
        self.inner.get_frozen_rows_count(sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.frozen_cols_calls.set(self.frozen_cols_calls.get() + 1);
        self.inner.get_frozen_columns_count(sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.inner.get_row_height(sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.inner.get_column_width(sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.inner.get_show_grid_lines(sheet)
    }
    fn get_show_row_headers(&self, sheet: u32) -> Option<bool> {
        self.show_row_headers_calls
            .set(self.show_row_headers_calls.get() + 1);
        self.inner.get_show_row_headers(sheet)
    }
    fn get_show_col_headers(&self, sheet: u32) -> Option<bool> {
        self.show_col_headers_calls
            .set(self.show_col_headers_calls.get() + 1);
        self.inner.get_show_col_headers(sheet)
    }
    fn get_show_selection(&self) -> bool {
        self.show_selection_calls
            .set(self.show_selection_calls.get() + 1);
        self.inner.get_show_selection()
    }
}

#[test]
fn frame_inputs_capture_reads_each_scalar_exactly_once() {
    let model = CountingModel {
        inner: TestModel::new().with_sheet(2).with_frozen(1, 3),
        ..CountingModel::default()
    };
    let theme = Rc::new(CanvasTheme::light());

    let result = FrameInputs::capture(&model, canvas_default(), 1.0, theme, 0);

    let inputs = result.expect("healthy model must capture successfully");
    assert_eq!(
        model.sheet_calls.get(),
        1,
        "selected sheet read exactly once"
    );
    assert_eq!(model.view_calls.get(), 1, "selected view read exactly once");
    assert_eq!(
        model.frozen_rows_calls.get(),
        1,
        "frozen row count read exactly once"
    );
    assert_eq!(
        model.frozen_cols_calls.get(),
        1,
        "frozen column count read exactly once"
    );
    assert_eq!(
        model.show_row_headers_calls.get(),
        1,
        "row-header visibility read exactly once"
    );
    assert_eq!(
        model.show_col_headers_calls.get(),
        1,
        "column-header visibility read exactly once"
    );
    assert_eq!(
        model.show_selection_calls.get(),
        1,
        "selection visibility read exactly once"
    );

    // Not just "it was read once" — read the right thing.
    assert_eq!(inputs.sheet(), 2);
    assert_eq!(inputs.frozen_rows(), 1);
    assert_eq!(inputs.frozen_cols(), 3);
    assert!(inputs.show_row_headers());
    assert!(inputs.show_col_headers());
    assert!(inputs.show_selection());
}

fn base_model() -> TestModel {
    TestModel::new().with_sheet(2).with_frozen(1, 1)
}

fn capture(model: &TestModel) -> Result<FrameInputs, FrameInputFailure> {
    FrameInputs::capture(
        model,
        canvas_default(),
        1.0,
        Rc::new(CanvasTheme::light()),
        0,
    )
}

#[test]
fn frame_inputs_capture_succeeds_on_a_healthy_model() {
    assert!(capture(&base_model()).is_ok());
}

#[test]
fn frame_inputs_failure_selected_sheet() {
    let model = base_model().with_capture_fail(FrameInputFailure::SelectedSheet);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::SelectedSheet)
    ));
}

#[test]
fn frame_inputs_failure_selected_view() {
    let model = base_model().with_capture_fail(FrameInputFailure::SelectedView);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::SelectedView)
    ));
}

// The standalone selected-sheet read and `CanvasView.sheet` are required to
// agree; a mismatch must hold the attempt rather than building a frame from
// one accessor's sheet and the other's coordinates.
#[test]
fn frame_inputs_failure_sheet_mismatch_on_divergent_selected_sheets() {
    let model = base_model().with_capture_fail(FrameInputFailure::SheetMismatch);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::SheetMismatch)
    ));
}

#[test]
fn frame_inputs_failure_frozen_rows() {
    let model = base_model().with_capture_fail(FrameInputFailure::FrozenRows);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::FrozenRows)
    ));
}

#[test]
fn frame_inputs_failure_frozen_columns() {
    let model = base_model().with_capture_fail(FrameInputFailure::FrozenColumns);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::FrozenColumns)
    ));
}

#[test]
fn frame_inputs_failure_row_header_visibility() {
    let model = base_model().with_capture_fail(FrameInputFailure::RowHeaderVisibility);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::RowHeaderVisibility)
    ));
}

#[test]
fn frame_inputs_failure_column_header_visibility() {
    let model = base_model().with_capture_fail(FrameInputFailure::ColumnHeaderVisibility);
    assert!(matches!(
        capture(&model),
        Err(FrameInputFailure::ColumnHeaderVisibility)
    ));
}
