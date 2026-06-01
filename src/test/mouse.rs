//! Tests for mouse input helpers: formula-reference drag and header span.

use crate::coord::{CellAddress, CellArea, SheetRange};
use iron_canvas_core::geometry::constants::{LAST_COLUMN, LAST_ROW};
use iron_canvas_core::types::ui::{Corner, RefZone, Side};

use crate::input::mouse::formula_ref::dragged_ref_range;
use crate::input::mouse::header_span::{Axis, full_header_span};

fn cell(row: i32, column: i32) -> CellAddress {
    CellAddress {
        sheet: 0,
        row,
        column,
    }
}

fn anchor(r1: i32, c1: i32, r2: i32, c2: i32) -> SheetRange {
    SheetRange::new(0, r1, c1, r2, c2)
}

// Body — the regression. B4:B6, grab inside at B5, drop two columns
// right at D5 must MOVE to D4:D6, not extend to B4:D6.
#[test]
fn body_translates_whole_rect() {
    let out = dragged_ref_range(anchor(4, 2, 6, 2), RefZone::Body, cell(5, 2), cell(5, 4));
    assert_eq!(out, anchor(4, 4, 6, 4));
}

// Body — clamping at the leading corner must shrink the trailing
// delta by the same amount so width/height stay constant.
#[test]
fn body_clamps_leading_corner_and_keeps_shape() {
    // A2:C4 (3 wide × 3 tall). Grab at B3, drop at A3 (one col left).
    // Then drop one MORE col left — c1 would go to 0, clamps to 1,
    // and c2 must follow: 3 - 1 = 2 (since c1 moved from 2 to 1).
    let out = dragged_ref_range(anchor(2, 2, 4, 4), RefZone::Body, cell(3, 3), cell(3, 1));
    assert_eq!(out, anchor(2, 1, 4, 3));
}

// Body — anchor stored un-normalized (B6:B4 as the user typed it)
// must drag identically to the normalized form.
#[test]
fn body_normalizes_inverted_anchor() {
    let out = dragged_ref_range(anchor(6, 2, 4, 2), RefZone::Body, cell(5, 2), cell(5, 4));
    assert_eq!(out, anchor(4, 4, 6, 4));
}

// Body — zero delta is a no-op (Excel ignores drop-on-origin, but
// the range math itself should still be the identity).
#[test]
fn body_zero_delta_is_identity() {
    let a = anchor(2, 2, 5, 5);
    let out = dragged_ref_range(a, RefZone::Body, cell(3, 3), cell(3, 3));
    assert_eq!(out, a);
}

#[test]
fn edge_right_extends_only_c2() {
    let out = dragged_ref_range(
        anchor(2, 2, 4, 4),
        RefZone::Edge(Side::Right),
        cell(3, 4),
        cell(3, 6),
    );
    assert_eq!(out, anchor(2, 2, 4, 6));
}

#[test]
fn edge_bottom_extends_only_r2() {
    let out = dragged_ref_range(
        anchor(2, 2, 4, 4),
        RefZone::Edge(Side::Bottom),
        cell(4, 3),
        cell(7, 3),
    );
    assert_eq!(out, anchor(2, 2, 7, 4));
}

#[test]
fn corner_bottom_right_resizes_both_axes() {
    let out = dragged_ref_range(
        anchor(2, 2, 4, 4),
        RefZone::Corner(Corner::BottomRight),
        cell(4, 4),
        cell(6, 7),
    );
    assert_eq!(out, anchor(2, 2, 6, 7));
}

#[test]
fn corner_top_left_resizes_both_axes() {
    let out = dragged_ref_range(
        anchor(3, 3, 5, 5),
        RefZone::Corner(Corner::TopLeft),
        cell(3, 3),
        cell(2, 1),
    );
    assert_eq!(out, anchor(2, 1, 5, 5));
}

// Shrink — BottomRight corner with cursor INSIDE the anchor must
// pull r2/c2 inward, keeping r1/c1 pinned at the opposite TopLeft.
// Anchor B2:E10, grab BR, drop at C3 → expect B2:C3.
#[test]
fn corner_bottom_right_shrinks_when_cursor_inside_anchor() {
    let out = dragged_ref_range(
        anchor(2, 2, 10, 5),
        RefZone::Corner(Corner::BottomRight),
        cell(10, 5),
        cell(3, 3),
    );
    assert_eq!(out, anchor(2, 2, 3, 3));
}

// Shrink — Right edge with cursor left of c2 must pull c2 inward
// and keep r1/r2 pinned (single-axis resize).
// Anchor B2:E10, grab Right edge, cursor at col 3 → expect B2:C10.
#[test]
fn edge_right_shrinks_when_cursor_left_of_c2() {
    let out = dragged_ref_range(
        anchor(2, 2, 10, 5),
        RefZone::Edge(Side::Right),
        cell(5, 5),
        cell(5, 3),
    );
    assert_eq!(out, anchor(2, 2, 10, 3));
}

// Cross-anchor — dragging TopLeft past the BottomRight degenerates
// to a single cell at the pinned (BR) anchor rather than flipping
// or producing an inverted range. Anchor B2:E10, grab TL, cursor
// at F12 (past BR) → expect E10:E10 (clamped collapse).
#[test]
fn corner_top_left_collapses_when_cursor_past_br() {
    let out = dragged_ref_range(
        anchor(2, 2, 10, 5),
        RefZone::Corner(Corner::TopLeft),
        cell(2, 2),
        cell(12, 6),
    );
    assert_eq!(out, anchor(10, 5, 10, 5));
}

// --- header_span tests ---

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
