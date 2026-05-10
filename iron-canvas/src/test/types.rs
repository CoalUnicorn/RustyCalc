#![allow(dead_code)]
#![allow(unused_imports)]

use crate::geometry::constants::{HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT};
use crate::chrome::FrozenRC;
use crate::geometry::prim::{Axis, Point};
use crate::layer::RenderOverlays;
use crate::renderer::PaneRegion;

#[test]
fn row_header_rect_pins_x_to_left_strip() {
    let rect = Axis::Row.header_rect(100, 20, HEADER_COL_WIDTH);
    assert_eq!(rect.top_left.x, HEADER_OFFSET);
    assert_eq!(rect.top_left.y, 100);
    assert_eq!(rect.width, HEADER_COL_WIDTH);
    assert_eq!(rect.height, 20);
}

#[test]
fn column_header_rect_pins_y_to_top_strip() {
    let rect = Axis::Column.header_rect(100, 20, HEADER_ROW_HEIGHT);
    assert_eq!(rect.top_left.x, 100);
    assert_eq!(rect.top_left.y, HEADER_OFFSET);
    assert_eq!(rect.width, 20);
    assert_eq!(rect.height, HEADER_ROW_HEIGHT);
}

#[test]
fn row_header_rect_thickness_maps_to_height() {
    let rect = Axis::Row.header_rect(100, 50, HEADER_COL_WIDTH);
    assert_eq!(rect.height, 50);
}

#[test]
fn column_header_rect_thickness_maps_to_width() {
    let rect = Axis::Column.header_rect(100, 50, HEADER_ROW_HEIGHT);
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

fn frozen(rows: i32, cols: i32, origin: Point) -> FrozenRC {
    FrozenRC {
        rows,
        cols,
        offset: origin,
    }
}

// #[test]
// fn pane_top_left_tags_both_bands_frozen() {
//     let p = PaneRegion::top_left();
//     assert_eq!(p.row_band, Band::Frozen);
//     assert_eq!(p.col_band, Band::Frozen);
// }

// #[test]
// fn pane_top_right_tags_frozen_rows_scroll_cols() {
//     let p = PaneRegion::top_right();
//     assert_eq!(p.row_band, Band::Frozen);
//     assert_eq!(p.col_band, Band::Scroll);
// }

// #[test]
// fn pane_bottom_left_tags_scroll_rows_frozen_cols() {
//     let p = PaneRegion::bottom_left();
//     assert_eq!(p.row_band, Band::Scroll);
//     assert_eq!(p.col_band, Band::Frozen);
// }

// #[test]
// fn pane_bottom_right_tags_both_bands_scroll() {
//     let p = PaneRegion::bottom_right();
//     assert_eq!(p.row_band, Band::Scroll);
//     assert_eq!(p.col_band, Band::Scroll);
// }

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
