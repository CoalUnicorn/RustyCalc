use iron_canvas_core::CanvasModel;
use ironcalc_base::UserModel;
use ironcalc_base::expressions::types::Area;

use crate::components::workbook::camera::extract_grid;
use crate::coord::SheetRange;

#[allow(clippy::expect_used)]
fn make_model() -> UserModel<'static> {
    UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("failed to create test model")
}

#[test]
fn extract_carries_values_and_dimensions() {
    let mut m = make_model();
    if let Err(e) = m.set_user_input(0, 1, 1, "hello") {
        panic!("setup failed: {e}");
    }
    if let Err(e) = m.set_user_input(0, 2, 2, "=1+1") {
        panic!("setup failed: {e}");
    }

    let grid = extract_grid(&m, SheetRange::new(0, 1, 1, 2, 2));

    assert_eq!(grid.column_count(), 2);
    assert_eq!(grid.row_count(), 2);
    assert_eq!(grid.cell_value(0, 0), Some("hello"));
    assert_eq!(grid.cell_value(1, 1), Some("2"));
    // headerless picture: both strips off
    assert_eq!(grid.get_show_row_headers(0), Some(false));
    assert_eq!(grid.get_show_col_headers(0), Some(false));
}

#[test]
fn extract_carries_styles() {
    let mut m = make_model();
    if let Err(e) = m.set_user_input(0, 1, 1, "styled") {
        panic!("setup failed: {e}");
    }
    // Apply bold via IronCalc's update_range_style path (same as the toolbar).
    let area = Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 1,
        height: 1,
    };
    if let Err(e) = m.update_range_style(&area, "font.b", "true") {
        panic!("style setup failed: {e}");
    }

    let grid = extract_grid(&m, SheetRange::new(0, 1, 1, 1, 1));
    let Some(style) = grid.cell_style(0, 0) else {
        panic!("expected a style on the bold cell");
    };
    assert!(style.font.bold);
}

// ==============================================================================
// events_touch_source — watch-set intersection tests
// ==============================================================================

#[cfg(test)]
mod watch_tests {
    use crate::components::workbook::camera::events_touch_source;
    use crate::coord::{CellAddress, SheetRange};
    use crate::events::{ContentEvent, FormatEvent};

    fn src() -> SheetRange {
        SheetRange::new(0, 2, 2, 5, 5)
    }

    #[test]
    fn cell_change_inside_source_touches() {
        let ev = ContentEvent::CellChanged {
            address: CellAddress {
                sheet: 0,
                row: 3,
                column: 3,
            },
            old_value: None,
            new_value: None,
        };
        assert!(events_touch_source(src(), &[ev], &[]));
    }

    #[test]
    fn cell_change_on_other_sheet_does_not_touch() {
        let ev = ContentEvent::CellChanged {
            address: CellAddress {
                sheet: 1,
                row: 3,
                column: 3,
            },
            old_value: None,
            new_value: None,
        };
        assert!(!events_touch_source(src(), &[ev], &[]));
    }

    #[test]
    fn overlapping_range_style_touches() {
        let ev = FormatEvent::RangeStyleChanged {
            area: SheetRange::new(0, 4, 4, 9, 9),
        };
        assert!(events_touch_source(src(), &[], &[ev]));
    }

    #[test]
    fn disjoint_range_change_does_not_touch() {
        let ev = ContentEvent::RangeChanged {
            sheet_area: SheetRange::new(0, 20, 20, 30, 30),
        };
        assert!(!events_touch_source(src(), &[ev], &[]));
    }

    // #5: a recalc can rewrite a source cell whose formula references *another*
    // sheet, so a CalculationUpdated naming only other sheets must still
    // re-extract. We can't see the source's cross-sheet deps from the range
    // alone, so the "never go stale" contract over-reports rather than miss it.
    #[test]
    fn calculation_update_on_other_sheet_still_touches() {
        let ev = ContentEvent::CalculationUpdated {
            affected_sheets: vec![1],
        };
        assert!(events_touch_source(src(), &[ev], &[]));
    }
}
