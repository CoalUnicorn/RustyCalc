use crate::geometry::constants::{HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT};
use crate::geometry::frame::frozen::FrozenRC;
use crate::geometry::frame::VisibleCells;
use crate::geometry::prim::{Axis, Point};
use crate::layer::RenderOverlays;
use crate::renderer::PaneRegion;

#[test]
fn row_header_rect_pins_x_to_left_strip() {
    let rect = Axis::Row.header_rect(100, 20);
    assert_eq!(rect.top_left.x, HEADER_OFFSET);
    assert_eq!(rect.top_left.y, 100);
    assert_eq!(rect.width, HEADER_COL_WIDTH);
    assert_eq!(rect.height, 20);
}

#[test]
fn column_header_rect_pins_y_to_top_strip() {
    let rect = Axis::Column.header_rect(100, 20);
    assert_eq!(rect.top_left.x, 100);
    assert_eq!(rect.top_left.y, HEADER_OFFSET);
    assert_eq!(rect.width, 20);
    assert_eq!(rect.height, HEADER_ROW_HEIGHT);
}

#[test]
fn row_header_rect_thickness_maps_to_height() {
    let rect = Axis::Row.header_rect(100, 50);
    assert_eq!(rect.height, 50);
}

#[test]
fn column_header_rect_thickness_maps_to_width() {
    let rect = Axis::Column.header_rect(100, 50);
    assert_eq!(rect.width, 50);
}

#[test]
fn row_strip_start_is_below_top_header() {
    assert_eq!(Axis::Row.strip_start(), HEADER_ROW_HEIGHT + HEADER_OFFSET);
}

#[test]
fn column_strip_start_is_right_of_left_header() {
    assert_eq!(Axis::Column.strip_start(), HEADER_COL_WIDTH + HEADER_OFFSET);
}

fn vis(rows: (i32, i32), cols: (i32, i32)) -> VisibleCells {
    VisibleCells {
        first: crate::CellRC {
            row: rows.0,
            column: cols.0,
        },
        last: crate::CellRC {
            row: rows.1,
            column: cols.1,
        },
    }
}

#[test]
fn row_visible_band_uses_first_last_row() {
    let v = vis((3, 17), (5, 12));
    let band = Axis::Row.visible_band(&v);
    assert_eq!(*band.start(), 3);
    assert_eq!(*band.end(), 17);
}

#[test]
fn column_visible_band_uses_first_last_column() {
    let v = vis((3, 17), (5, 12));
    let band = Axis::Column.visible_band(&v);
    assert_eq!(*band.start(), 5);
    assert_eq!(*band.end(), 12);
}

fn frozen(rows: i32, cols: i32, origin: Point) -> FrozenRC {
    FrozenRC {
        rows,
        cols,
        offset: origin,
    }
}

#[test]
fn pane_top_left_origin_is_pinned_to_header_corner() {
    let frc = frozen(2, 3, Point { x: 200, y: 100 });
    let p = PaneRegion::top_left(&frc);
    assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
}

#[test]
fn pane_top_right_origin_uses_frozen_x_and_header_y() {
    let frc = frozen(2, 3, Point { x: 200, y: 100 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::top_right(&frc, &v);
    assert_eq!(p.origin.x, 200);
    assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
    assert_eq!(*p.cols.start(), 4);
}

#[test]
fn pane_bottom_left_origin_uses_header_x_and_frozen_y() {
    let frc = frozen(2, 3, Point { x: 200, y: 100 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::bottom_left(&frc, &v);
    assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(p.origin.y, 100);
    assert_eq!(*p.rows.start(), 3);
}

#[test]
fn pane_bottom_right_origin_matches_frozen_offset() {
    let frc = frozen(2, 3, Point { x: 200, y: 100 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::bottom_right(&frc, &v);
    assert_eq!(p.origin.x, 200);
    assert_eq!(p.origin.y, 100);
}

#[test]
fn render_overlays_default_equals_itself() {
    let a = RenderOverlays::default();
    let b = RenderOverlays::default();
    assert!(a == b);
}

#[test]
fn render_overlays_changed_point_range_is_not_equal() {
    use crate::RCRange;
    let a = RenderOverlays::default();
    let mut b = RenderOverlays {
        point_range: Some(RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        }),
        extend_to: None,
        clipboard: None,
        formula_refs: vec![],
    };
    b.point_range = Some(RCRange {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 2,
    });
    assert!(a != b);
}

#[test]
fn render_overlays_same_point_range_is_equal() {
    use crate::RCRange;
    let range = Some(RCRange {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 2,
    });
    let a = RenderOverlays {
        point_range: range,
        ..Default::default()
    };
    let b = RenderOverlays {
        point_range: range,
        ..Default::default()
    };
    assert!(a == b);
}
