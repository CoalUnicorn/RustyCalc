#![allow(clippy::unwrap_used)]

use ironcalc_base::expressions::types::CellReferenceRC;

use crate::model::ArrowKey;

use crate::coord::*;

#[test]
fn contains_includes_corners() {
    let a = CellArea {
        r1: 1,
        c1: 1,
        r2: 3,
        c2: 3,
    };
    assert!(a.contains(1, 1), "top-left");
    assert!(a.contains(3, 3), "bottom-right");
    assert!(!a.contains(4, 1), "outside");
}

#[test]
fn contains_single_cell_area() {
    let a = CellArea::from_cell(5, 7);
    assert!(a.contains(5, 7));
    assert!(!a.contains(5, 8));
}

#[test]
fn normalized_swaps_inverted_coords() {
    let a = CellArea {
        r1: 4,
        c1: 3,
        r2: 1,
        c2: 1,
    };
    assert_eq!(
        a.normalized(),
        CellArea {
            r1: 1,
            c1: 1,
            r2: 4,
            c2: 3
        }
    );
}

#[test]
fn to_sheet_area_produces_single_cell() {
    let addr = CellAddress {
        sheet: 2,
        row: 4,
        column: 6,
    };
    let sa = addr.to_sheet_area();
    assert_eq!(sa.sheet, 2);
    assert_eq!(sa.area, CellArea::from_cell(4, 6));
    assert!(sa.area.is_single_cell());
}

fn ctx_a1() -> CellReferenceRC {
    CellReferenceRC {
        sheet: "Sheet1".into(),
        row: 1,
        column: 1,
    }
}

fn editing_a1() -> CellAddress {
    CellAddress {
        sheet: 0,
        row: 1,
        column: 1,
    }
}

// Relative Node fields store offsets from ctx: zero-offset from A1 -> "A1".
#[test]
fn refnode_a1_roundtrip() {
    let n = RefNode::cell(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "A1");
}

// Absolute Node fields store the final 1-based coordinate directly.
#[test]
fn refnode_absolute_a1_roundtrip() {
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "$A$1");
}

#[test]
fn refnode_cross_sheet_range() {
    let n = RefNode::range(
        1,
        Some("Sheet2".into()),
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
        2,
        1,
        Absolute {
            row: false,
            column: false,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "Sheet2!A1:B3");
}

#[test]
fn refnode_quoted_sheet_name() {
    let n = RefNode::cell(
        1,
        Some("Space Sheet".into()),
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "'Space Sheet'!A1");
}

#[test]
fn refnode_rc_format_absolute_is_r1c1() {
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    assert_eq!(n.to_rc(), "R1C1");
}

// Relative ref: stored fields are deltas; area() must add editing coords.
#[test]
fn refnode_area_relative_resolves_with_editing() {
    let n = RefNode::cell(
        3,
        None,
        4,
        6,
        Absolute {
            row: false,
            column: false,
        },
    );
    let resolved = n.area(&editing_a1());
    assert_eq!(resolved, SheetRange::from_cell(3, 5, 7));
}

// Absolute ref: stored fields are already absolute; editing is ignored.
#[test]
fn refnode_area_absolute_ignores_editing() {
    let n = RefNode::cell(
        3,
        None,
        5,
        7,
        Absolute {
            row: true,
            column: true,
        },
    );
    let editing_far_away = CellAddress {
        sheet: 0,
        row: 100,
        column: 100,
    };
    assert_eq!(n.area(&editing_far_away), SheetRange::from_cell(3, 5, 7));
}

// Range: each corner resolved independently via its own absolute flags.
#[test]
fn refnode_area_range_mixed_flags() {
    // Anchor absolute at A1, trailing relative at delta (2,1) from editing.
    let n = RefNode::range(
        2,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
        2,
        1,
        Absolute {
            row: false,
            column: false,
        },
    );
    let editing = CellAddress {
        sheet: 0,
        row: 1,
        column: 1,
    };
    assert_eq!(n.area(&editing), SheetRange::new(2, 1, 1, 3, 2));
}

#[test]
fn from_cell_area_same_sheet_omits_name() {
    let area = SheetRange::from_cell(0, 1, 1);
    let n = RefNode::from_cell_area(area, editing_a1(), "Sheet1");
    assert_eq!(n.to_localized(&ctx_a1()), "A1");
}

#[test]
fn from_cell_area_cross_sheet_qualifies() {
    let area = SheetRange::from_cell(1, 1, 1);
    let n = RefNode::from_cell_area(area, editing_a1(), "Sheet2");
    assert_eq!(n.to_localized(&ctx_a1()), "Sheet2!A1");
}

#[test]
fn from_cell_area_relative_offset() {
    let area = SheetRange::from_cell(0, 5, 3);
    let editing = CellAddress {
        sheet: 0,
        row: 2,
        column: 2,
    };
    let ctx = CellReferenceRC {
        sheet: "Sheet1".into(),
        row: 2,
        column: 2,
    };
    let n = RefNode::from_cell_area(area, editing, "Sheet1");
    assert_eq!(n.to_localized(&ctx), "C5");
}

// Plain arrow: whole reference moves. Relative Node fields store deltas
// from ctx A1, so +1 to the row field shifts the resolved coord one row
// down — matching ironcalc's stringify semantics.
#[test]

fn extend_trailing_single_cell_arrow_down() {
    let n = RefNode::cell(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
    assert_eq!(moved.to_localized(&ctx_a1()), "A2");
}

// Absolute flags survive the shift; stored coord increments directly.
#[test]
fn extend_trailing_preserves_absolute() {
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
    assert_eq!(moved.to_localized(&ctx_a1()), "$A$2");
}

// Range variant: plain arrow drops the anchor, leaving a single cell at
// trailing + delta. Matches Excel's "plain arrow forgets the range".
#[test]
fn extend_trailing_range_collapses_to_trailing() {
    let n = RefNode::range(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
        1,
        1,
        Absolute {
            row: false,
            column: false,
        },
    );
    let moved = n.extend_trailing(&ArrowKey::from_str("ArrowRight").unwrap());
    assert_eq!(moved.to_localized(&ctx_a1()), "C2");
}

// Sheet qualification is part of the preserved metadata.
#[test]
fn extend_trailing_preserves_sheet_cross_sheet() {
    let n = RefNode::cell(
        1,
        Some("Sheet2".into()),
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let moved = n.extend_trailing(&ArrowKey::from_str("ArrowDown").unwrap());
    let cell_ref = CellReferenceRC {
        sheet: "Sheet1".into(),
        row: 1,
        column: 1,
    };
    assert_eq!(moved.to_localized(&cell_ref), "Sheet2!A2");
}

// Shift+arrow: anchor stays, trailing moves. Single cell -> promoted to range.
#[test]
fn extend_with_anchor_promotes_to_range() {
    let n = RefNode::cell(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
    assert_eq!(grown.to_localized(&ctx_a1()), "A1:A2");
}

#[test]
fn extend_with_anchor_grows_range() {
    let n = RefNode::range(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
        1,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
    assert_eq!(grown.to_localized(&ctx_a1()), "A1:A3");
}

#[test]
fn extend_with_anchor_absolute_flags_survive() {
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowRight").unwrap());
    assert_eq!(grown.to_localized(&ctx_a1()), "$A$1:$B$1");
}

#[test]
fn extend_with_anchor_mixed_absolute_flag_promotion() {
    let n = RefNode::cell(
        0,
        None,
        0,
        1,
        Absolute {
            row: false,
            column: true,
        },
    );
    let grown = n.extend_with_anchor(&ArrowKey::from_str("ArrowDown").unwrap());
    assert_eq!(grown.to_localized(&ctx_a1()), "$A1:$A2");
}

/// Regression: `Absolute` named fields prevent the `(row_bool, col_bool)`
/// swap that the old `cell(…, true, false)` signature was prone to.
#[test]
fn absolute_flags_not_swapped() {
    // Column-absolute (1 = A), row-relative (delta 0 from ctx row 1) → $A1
    let n = RefNode::cell(
        0,
        None,
        0,
        1,
        Absolute {
            row: false,
            column: true,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "$A1");

    // Row-absolute (1), column-relative (delta 0 from ctx col 1) → A$1
    let n = RefNode::cell(
        0,
        None,
        1,
        0,
        Absolute {
            row: true,
            column: false,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "A$1");

    // Both relative → A1
    let n = RefNode::cell(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "A1");

    // Both absolute → $A$1
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    assert_eq!(n.to_localized(&ctx_a1()), "$A$1");
}

// Click-to-replace on the same sheet: coordinates move, `$` flags survive,
// no sheet prefix appears.
#[test]
fn relocate_to_same_sheet_preserves_flags() {
    let n = RefNode::cell(
        0,
        None,
        1,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    let moved = n.relocate_to(5, 2, &editing_a1(), 0, "Sheet1");
    assert_eq!(moved.to_localized(&ctx_a1()), "$B$5");
}

// Cross-sheet click-to-replace: the ref adopts the clicked sheet and gains
// qualification because it differs from the edit origin (sheet 0).
#[test]
fn relocate_to_cross_sheet_adopts_clicked_sheet() {
    let n = RefNode::cell(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let moved = n.relocate_to(5, 2, &editing_a1(), 1, "Sheet2");
    assert_eq!(moved.to_localized(&ctx_a1()), "Sheet2!B5");
}

// Clicking back on the origin sheet drops a stale qualification: replace
// means replace, the old ref's `Sheet2!` does not survive.
#[test]
fn relocate_to_origin_sheet_drops_qualification() {
    let n = RefNode::cell(
        1,
        Some("Sheet2".into()),
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
    );
    let moved = n.relocate_to(5, 2, &editing_a1(), 0, "Sheet1");
    assert_eq!(moved.to_localized(&ctx_a1()), "B5");
}

// A range collapses to a cell inheriting the trailing corner's flags,
// combined with cross-sheet adoption.
#[test]
fn relocate_to_cross_sheet_range_collapses_with_trailing_flags() {
    let n = RefNode::range(
        0,
        None,
        0,
        0,
        Absolute {
            row: false,
            column: false,
        },
        2,
        1,
        Absolute {
            row: true,
            column: true,
        },
    );
    let moved = n.relocate_to(5, 2, &editing_a1(), 1, "Sheet2");
    assert_eq!(moved.to_localized(&ctx_a1()), "Sheet2!$B$5");
}
