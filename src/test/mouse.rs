//! Tests for the header-span derivation used by header clicks and resize.

use crate::coord::{CellArea, LAST_COLUMN, LAST_ROW};

use crate::input::mouse::header_span::{Axis, full_header_span};

fn full_col_area(c1: i32, c2: i32) -> CellArea {
    CellArea {
        r1: 1,
        c1,
        r2: LAST_ROW,
        c2,
    }
}

fn full_row_area(r1: i32, r2: i32) -> CellArea {
    CellArea {
        r1,
        c1: 1,
        r2,
        c2: LAST_COLUMN,
    }
}

#[test]
fn span_is_selection_range_when_col_inside_full_column_selection() {
    let area = full_col_area(2, 4);
    assert_eq!(full_header_span(area, 3, Axis::Col), (2, 4));
}

#[test]
fn span_is_single_when_col_outside_selection() {
    let area = full_col_area(2, 4);
    assert_eq!(full_header_span(area, 9, Axis::Col), (9, 9));
}

#[test]
fn span_is_single_when_selection_not_full_height() {
    // r2=10, not LAST_ROW — not a full-column strip
    let area = CellArea {
        r1: 1,
        c1: 2,
        r2: 10,
        c2: 4,
    };
    assert_eq!(full_header_span(area, 3, Axis::Col), (3, 3));
}

#[test]
fn span_row_axis_mirrors_column_logic() {
    let area = full_row_area(2, 4);
    assert_eq!(full_header_span(area, 3, Axis::Row), (2, 4));
    assert_eq!(full_header_span(area, 9, Axis::Row), (9, 9));
}

// normalized() contract — an inverted full-column selection (c1 > c2)
// passed to full_header_span must still return the correct ordered span.
// col 3 is inside [2,4] regardless of storage order.
#[test]
fn span_normalizes_inverted_col_area() {
    let area = full_col_area(4, 2); // c1=4, c2=2 — intentionally inverted
    assert_eq!(full_header_span(area, 3, Axis::Col), (2, 4));
}
