use crate::coord::CellArea;
use crate::model::clipboard_bridge::*;
use ironcalc_base::UserModel;
use ironcalc_base::types::BorderStyle;

// CellRange::tile_reps_of

#[test]
fn tile_reps_single_cell_into_range() {
    let src = CellArea {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 1,
    };
    let dst = CellArea {
        r1: 1,
        c1: 1,
        r2: 3,
        c2: 4,
    };
    assert_eq!(dst.tile_reps_of(src), Some((3, 4)));
}

#[test]
fn tile_reps_exact_multiple() {
    let src = CellArea {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 3,
    };
    let dst = CellArea {
        r1: 1,
        c1: 1,
        r2: 4,
        c2: 6,
    };
    assert_eq!(dst.tile_reps_of(src), Some((2, 2)));
}

#[test]
fn tile_reps_non_multiple_returns_none() {
    let src = CellArea {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 2,
    };
    let dst = CellArea {
        r1: 1,
        c1: 1,
        r2: 3,
        c2: 3,
    };
    assert_eq!(dst.tile_reps_of(src), None);
}

#[test]
fn tile_reps_same_size_returns_none() {
    let src = CellArea {
        r1: 1,
        c1: 1,
        r2: 2,
        c2: 2,
    };
    assert_eq!(src.tile_reps_of(src), None);
}

// AppClipboard::capture roundtrip

#[allow(clippy::expect_used)]
#[test]
fn capture_roundtrip() {
    let model = UserModel::new_empty("Sheet1", "en", "UTC", "en").expect("create test model");
    let cb = model.copy_to_clipboard().expect("copy empty range");
    let app = AppClipboard::capture(&cb).expect("capture roundtrip");
    assert_eq!(app.sheet, 0);
}

// BorderArea construction

#[test]
fn make_border_area_all_thin_black() {
    let ba = make_border_area(
        BorderKind::All,
        BorderStyle::Thin,
        Some("#000000".to_owned()),
    );
    // If this didn't panic, the serde roundtrip succeeded.
    let _ = ba;
}

#[test]
fn make_border_area_none() {
    let ba = make_border_area(BorderKind::None, BorderStyle::Thin, None);
    let _ = ba;
}
