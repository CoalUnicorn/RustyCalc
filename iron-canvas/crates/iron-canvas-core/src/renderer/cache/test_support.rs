//! Test fixtures for the fingerprint tree and the modules that plan against it.
//!
//! Compiled only under `cfg(test)`. `layout` builds a real `GridLayout` through
//! `Chrome::next`, `dense` fills one dense fetch bundle, and `build` runs the
//! production `FingerprintState::build_candidate` path — no test hand-writes
//! tree internals.

use std::rc::Rc;

use super::fingerprint::{FingerprintState, GridFingerprint};
use crate::FrameInputs;
use crate::chrome::{Chrome, FramePath, GridLayout};
use crate::geometry::{CanvasMetrics, CanvasSize};
use crate::model_adapter::{CanvasModel, CanvasView, CellContentQuery};
use crate::renderer::prepared::FetchedCells;
use crate::style::{CellKind, CellStyle};
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

struct LayoutModel {
    top: i32,
    left: i32,
    frozen_rows: i32,
    frozen_cols: i32,
}

impl CellContentQuery for LayoutModel {
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
        Fetched::Absent
    }

    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
        Fetched::Absent
    }

    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Fetched<String> {
        Fetched::Absent
    }
}

impl CanvasModel for LayoutModel {
    fn get_selected_sheet(&self) -> Option<u32> {
        Some(0)
    }

    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: self.top,
            column: self.left,
            selection: RCRange::from_cell(self.top, self.left),
            top_row: self.top,
            left_column: self.left,
        })
    }

    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_rows)
    }

    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_cols)
    }

    fn get_row_height(&self, _: u32, _: i32) -> Fetched<f64> {
        Fetched::Value(20.0)
    }

    fn get_column_width(&self, _: u32, _: i32) -> Fetched<f64> {
        Fetched::Value(60.0)
    }

    fn get_show_grid_lines(&self, _: u32) -> Fetched<bool> {
        Fetched::Value(true)
    }
}

pub(crate) fn layout(top: i32, left: i32, frozen_rows: i32, frozen_cols: i32) -> GridLayout {
    let model = LayoutModel {
        top,
        left,
        frozen_rows,
        frozen_cols,
    };
    let inputs = FrameInputs::capture(
        &model,
        CanvasMetrics::new(CanvasSize { w: 420.0, h: 260.0 }, 1.0)
            .expect("test canvas metrics are valid"),
        Rc::new(CanvasTheme::light()),
        0,
    )
    .unwrap();
    Chrome::next(None, &model, &inputs, FramePath::Fresh).grid_layout()
}

pub(crate) fn dense(range: RCRange) -> FetchedCells {
    let mut styles = Vec::new();
    let mut values = Vec::new();
    let mut cell_types = Vec::new();
    let mut decorations = Vec::new();
    for row in range.rows() {
        for col in range.columns() {
            styles.push(Fetched::Value(CellStyle::default()));
            values.push(Fetched::Value(format!("{row}:{col}")));
            cell_types.push(Fetched::Value(CellKind::Text));
            decorations.push(Fetched::Absent);
        }
    }
    FetchedCells::from_parts(styles, values, cell_types, decorations)
}

fn bundles(layout: GridLayout) -> [Option<FetchedCells>; 4] {
    let mut bundles = std::array::from_fn(|_| None);
    for segment in layout.segments() {
        bundles[segment.region().index()] = Some(dense(segment.range()));
    }
    bundles
}

pub(crate) fn build(layout: GridLayout) -> GridFingerprint {
    let bundles = bundles(layout);
    let references = std::array::from_fn(|index| bundles[index].as_ref());
    FingerprintState::default().build_candidate(layout, &references)
}
