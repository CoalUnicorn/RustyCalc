use crate::coord::CellArea;
use crate::scene::{clamp_viewport_origin, clipboard_range_on_sheet};
use iron_canvas_core::CellCoord;

#[test]
fn viewport_clamp_keeps_a_legal_live_origin() {
    let requested = CellCoord { row: 18, col: 7 };

    assert_eq!(clamp_viewport_origin(requested, 3, 2), requested);
}

#[test]
fn viewport_clamp_moves_the_origin_past_frozen_bands() {
    assert_eq!(
        clamp_viewport_origin(CellCoord { row: 2, col: 1 }, 4, 3),
        CellCoord { row: 5, col: 4 }
    );
}

#[test]
fn clipboard_overlay_only_matches_its_source_sheet() {
    let range = CellArea {
        r1: 2,
        c1: 3,
        r2: 5,
        c2: 7,
    };

    assert_eq!(clipboard_range_on_sheet(1, range, 1), Some(range.into()));
    assert_eq!(clipboard_range_on_sheet(1, range, 0), None);
}
