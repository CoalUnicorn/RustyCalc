use crate::model::{CssColor, RCRange};

#[test]
fn normalized_swaps_corners_when_inverted() {
    let r = RCRange {
        r1: 5,
        c1: 7,
        r2: 2,
        c2: 3,
    }
    .normalized();
    assert_eq!(
        r,
        RCRange {
            r1: 2,
            c1: 3,
            r2: 5,
            c2: 7
        }
    );
}

#[test]
fn normalized_is_idempotent_on_already_ordered() {
    let r = RCRange {
        r1: 1,
        c1: 2,
        r2: 3,
        c2: 4,
    };
    assert_eq!(r.normalized(), r);
}

#[test]
fn width_and_height_are_inclusive() {
    let r = RCRange {
        r1: 2,
        c1: 5,
        r2: 4,
        c2: 8,
    };
    assert_eq!(r.height(), 3);
    assert_eq!(r.width(), 4);
}

#[test]
fn is_single_cell_only_when_corners_match() {
    assert!(RCRange::from_cell(7, 9).is_single_cell());
    assert!(!RCRange {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 2
    }
    .is_single_cell());
}

#[test]
fn contains_respects_inclusive_bounds() {
    let r = RCRange {
        r1: 2,
        c1: 3,
        r2: 4,
        c2: 5,
    };
    assert!(r.contains(2, 3));
    assert!(r.contains(4, 5));
    assert!(r.contains(3, 4));
    assert!(!r.contains(1, 3));
    assert!(!r.contains(5, 5));
}

#[test]
fn cells_walks_row_major() {
    let r = RCRange {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 2,
    };
    let cells: Vec<_> = r.cells().collect();
    assert_eq!(cells, vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
}

#[test]
fn from_array_maps_in_order_r1_c1_r2_c2() {
    let r: RCRange = [3, 5, 7, 9].into();
    assert_eq!(
        r,
        RCRange {
            r1: 3,
            c1: 5,
            r2: 7,
            c2: 9
        }
    );
}

#[test]
fn with_sheet_attaches_sheet_id() {
    let area = RCRange::from_cell(2, 3).with_sheet(7);
    assert_eq!(area.sheet, 7);
    assert_eq!(area.range, RCRange::from_cell(2, 3));
}

#[test]
fn css_color_empty_string_falls_back_to_black() {
    assert_eq!(CssColor::new("").as_str(), "#000000");
}

#[test]
fn css_color_lowercases_hex_input() {
    assert_eq!(CssColor::new("#FF00AA").as_str(), "#ff00aa");
}
