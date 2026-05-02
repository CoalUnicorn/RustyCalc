use crate::geometry::Axis;
use crate::renderer::PaneRegion;
use crate::{
    FrozenRC, Point, RenderOverlays, VisibleCells, HEADER_COL_WIDTH, HEADER_OFFSET,
    HEADER_ROW_HEIGHT,
};

#[test]
fn row_header_rect_pins_x_to_left_strip() {
    let rect = Axis::Row.header_rect(100.0, 20.0);
    assert_eq!(rect.top_left.x, HEADER_OFFSET);
    assert_eq!(rect.top_left.y, 100.0);
    assert_eq!(rect.width, HEADER_COL_WIDTH);
    assert_eq!(rect.height, 20.0);
}

#[test]
fn column_header_rect_pins_y_to_top_strip() {
    let rect = Axis::Column.header_rect(100.0, 20.0);
    assert_eq!(rect.top_left.x, 100.0);
    assert_eq!(rect.top_left.y, HEADER_OFFSET);
    assert_eq!(rect.width, 20.0);
    assert_eq!(rect.height, HEADER_ROW_HEIGHT);
}

#[test]
fn row_header_rect_thickness_maps_to_height() {
    let rect = Axis::Row.header_rect(100.0, 50.0);
    assert_eq!(rect.height, 50.0);
}

#[test]
fn column_header_rect_thickness_maps_to_width() {
    let rect = Axis::Column.header_rect(100.0, 50.0);
    assert_eq!(rect.width, 50.0);
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
        first: crate::geometry::CellRC {
            row: rows.0,
            column: cols.0,
        },
        last: crate::geometry::CellRC {
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

fn frozen(rows: Option<(i32, i32)>, cols: Option<(i32, i32)>, origin: Point) -> FrozenRC {
    FrozenRC {
        row_band: rows.map(|(s, e)| s..=e),
        col_band: cols.map(|(s, e)| s..=e),
        offset: origin,
    }
}

#[test]
fn pane_top_left_origin_is_pinned_to_header_corner() {
    let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
    let p = PaneRegion::top_left(&frc);
    assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
}

#[test]
fn pane_top_right_origin_uses_frozen_x_and_header_y() {
    let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::top_right(&frc, &v);
    assert_eq!(p.origin.x, 200.0);
    assert_eq!(p.origin.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
    assert_eq!(*p.cols.start(), 4);
}

#[test]
fn pane_bottom_left_origin_uses_header_x_and_frozen_y() {
    let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::bottom_left(&frc, &v);
    assert_eq!(p.origin.x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(p.origin.y, 100.0);
    assert_eq!(*p.rows.start(), 3);
}

#[test]
fn pane_bottom_right_origin_matches_frozen_offset() {
    let frc = frozen(Some((1, 2)), Some((1, 3)), Point { x: 200.0, y: 100.0 });
    let v = vis((3, 9), (4, 11));
    let p = PaneRegion::bottom_right(&frc, &v);
    assert_eq!(p.origin.x, 200.0);
    assert_eq!(p.origin.y, 100.0);
}

#[test]
fn render_overlays_default_equals_itself() {
    let a = RenderOverlays::default();
    let b = RenderOverlays::default();
    assert!(a == b);
}

#[test]
fn render_overlays_changed_point_range_is_not_equal() {
    use crate::model::RCRange;
    let a = RenderOverlays::default();
    let mut b = RenderOverlays::default();
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
    use crate::model::RCRange;
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

#[test]
fn render_overlays_changed_selection_is_not_equal() {
    use crate::geometry::{PixelRect, Point};
    let a = RenderOverlays::default();
    let b = RenderOverlays {
        selection: Some(PixelRect {
            top_left: Point { x: 10.0, y: 10.0 },
            width: 80.0,
            height: 20.0,
        }),
        ..Default::default()
    };
    assert!(a != b);
}

#[test]
fn render_overlays_cleared_selection_equals_default() {
    let a = RenderOverlays::default();
    let b = RenderOverlays {
        selection: None,
        ..Default::default()
    };
    assert!(a == b);
}
