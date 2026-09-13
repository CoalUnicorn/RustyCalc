#![cfg(any(feature = "svg", feature = "pdf"))]

use std::cell::Cell;
use std::rc::Rc;

use iron_canvas_core::{
    CanvasMetricError, CanvasModel, CanvasSize, CanvasTheme, CanvasView, CellContentQuery,
    CellKind, CellStyle, Fetched, RCRange,
};
use iron_canvas_export::ExportError;

#[derive(Clone, Copy)]
enum Failure {
    None,
    Sheet,
    Cells,
}

struct Model {
    failure: Cell<Failure>,
    reads: Cell<usize>,
}

impl CellContentQuery for Model {
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
        Fetched::Absent
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
        Fetched::Absent
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Fetched<String> {
        if matches!(self.failure.get(), Failure::Cells) {
            Fetched::BridgeFailed
        } else {
            Fetched::Absent
        }
    }
}

impl CanvasModel for Model {
    fn get_selected_sheet(&self) -> Option<u32> {
        self.reads.set(self.reads.get() + 1);
        if matches!(self.failure.get(), Failure::Sheet) {
            None
        } else {
            Some(0)
        }
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from_cell(1, 1),
            top_row: 1,
            left_column: 1,
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _: u32, _: i32) -> Fetched<f64> {
        Fetched::Absent
    }
    fn get_column_width(&self, _: u32, _: i32) -> Fetched<f64> {
        Fetched::Absent
    }
    fn get_show_grid_lines(&self, _: u32) -> Fetched<bool> {
        Fetched::Absent
    }
}

fn assert_exports(model: &Rc<Model>, size: CanvasSize, expected: Result<(), ExportError>) {
    #[cfg(feature = "svg")]
    assert_eq!(
        iron_canvas_export::SvgSurface::render(model.clone(), &CanvasTheme::light(), size)
            .map(|_| ()),
        expected
    );
    #[cfg(feature = "pdf")]
    assert_eq!(
        iron_canvas_export::PdfSurface::render(model.clone(), &CanvasTheme::light(), size)
            .map(|_| ()),
        expected
    );
}

#[test]
fn invalid_metrics_fail_before_model_reads() {
    let model = Rc::new(Model {
        failure: Cell::new(Failure::None),
        reads: Cell::new(0),
    });
    for w in [f64::NAN, f64::INFINITY, -1.0, f64::from(i32::MAX) + 1.0] {
        assert_exports(
            &model,
            CanvasSize { w, h: 200.0 },
            Err(ExportError::Metrics(CanvasMetricError::Size)),
        );
    }
    assert_eq!(model.reads.get(), 0);
}

#[test]
fn held_export_returns_error_and_recovered_model_exports() {
    let model = Rc::new(Model {
        failure: Cell::new(Failure::None),
        reads: Cell::new(0),
    });
    let size = CanvasSize { w: 300.0, h: 200.0 };
    for failure in [Failure::Sheet, Failure::Cells] {
        model.failure.set(failure);
        assert_exports(&model, size, Err(ExportError::RetryRequired));
        model.failure.set(Failure::None);
        assert_exports(&model, size, Ok(()));
    }
}
