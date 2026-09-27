//! Merged-range conversion and the `CanvasModel::get_merged_ranges` adapter
//! contract.

use iron_canvas_core::{CanvasModel, RCRange};
use iron_canvas_ironcalc::IronCalcModel;
use iron_canvas_ironcalc::convert::merged_range_to_core;
use ironcalc_base::UserModel;
use ironcalc_base::expressions::types::Area;
use ironcalc_base::types as ic;

fn new_model() -> UserModel<'static> {
    match UserModel::new_empty("wb", "en", "UTC", "en") {
        Ok(m) => m,
        Err(e) => panic!("empty workbook must construct: {e}"),
    }
}

fn merged(row: i32, column: i32, width: i32, height: i32) -> ic::MergedCell {
    ic::MergedCell {
        row,
        column,
        width,
        height,
    }
}

#[test]
fn a_merged_cell_converts_to_inclusive_bounds() {
    assert_eq!(
        merged_range_to_core(merged(2, 3, 3, 2)),
        Some(RCRange {
            r1: 2,
            c1: 3,
            r2: 3,
            c2: 5,
        })
    );
    // A single cell is the degenerate case: width and height of 1.
    assert_eq!(
        merged_range_to_core(merged(7, 7, 1, 1)),
        Some(RCRange::from_cell(7, 7))
    );
}

#[test]
fn non_positive_extent_and_overflow_return_none() {
    assert_eq!(merged_range_to_core(merged(1, 1, 0, 2)), None);
    assert_eq!(merged_range_to_core(merged(1, 1, 2, 0)), None);
    assert_eq!(merged_range_to_core(merged(1, 1, -1, 2)), None);
    // The overflow is on the axis the extent spans: `height` extends the row,
    // `width` extends the column.
    assert_eq!(merged_range_to_core(merged(i32::MAX, 1, 1, 2)), None);
    assert_eq!(merged_range_to_core(merged(1, i32::MAX, 2, 1)), None);
}

/// The adapter reads the engine's merged list: a real `merge_cells` shows up,
/// and unmerging clears it.
#[test]
fn the_adapter_reports_the_engine_merged_list() {
    let mut m = new_model();
    let area = Area {
        sheet: 0,
        row: 2,
        column: 2,
        width: 3,
        height: 2,
    };
    m.merge_cells(&area).expect("merge must succeed");

    let model = IronCalcModel(m);
    let ranges = model
        .get_merged_ranges(0)
        .expect("a healthy engine answers the merge list");
    assert_eq!(
        ranges,
        vec![RCRange {
            r1: 2,
            c1: 2,
            r2: 3,
            c2: 4,
        }]
    );

    let mut m = model.0;
    m.unmerge_cells(&area).expect("unmerge must succeed");
    let model = IronCalcModel(m);
    assert_eq!(model.get_merged_ranges(0), Some(Vec::new()));
}
