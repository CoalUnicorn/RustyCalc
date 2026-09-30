//! Merge-aware autofill geometry.
//!
//! The engine owns the autofill acceptance rule: it tiles the source's merged
//! cells into the fill target and replaces the merges already there, and it
//! rejects a merge that the fill boundary cuts. The host must therefore show a
//! ghost whose extent the engine can actually accept — a ghost that stops
//! inside a merge promises a fill that cannot happen.
//!
//! Snapping is deliberately outward, to the merge's far edge along the fill
//! axis, because that is the extent the engine accepts (the whole merge is
//! replaced, not cut) and the behaviour Excel shows.

use ironcalc_base::types::MergedCell;

use leptos::prelude::WithValue;

use crate::coord::CellArea;
use crate::state::ModelStore;
use iron_canvas_core::AutofillTarget;

/// The one fill target the host both previews and submits.
///
/// The ghost and the commit must agree, or the preview promises an operation the
/// engine rejects: `IronCalc` refuses a fill whose boundary cuts a merged cell.
/// Both callers therefore resolve the target through this function.
pub fn resolved_fill_target(model: ModelStore, to_row: i32, to_col: i32) -> AutofillTarget {
    model.with_value(|m| {
        let view = m.get_selected_view();
        let merges = m.get_merged_cells(view.sheet).unwrap_or_default();
        snap_autofill_target(
            CellArea::from(view.range).normalized(),
            to_row,
            to_col,
            &merges,
        )
    })
}

/// Snap the autofill drag target out of any merge it lands inside.
///
/// `source` is the selection being filled; `to_row` / `to_col` are the raw
/// physical cell under the pointer. The fill axis is chosen exactly as the
/// commit path chooses it (a target outside the selection's rows fills rows,
/// otherwise columns), and the perpendicular axis is pinned to the selection so
/// the drawn ghost cannot be wider than the fill.
pub fn snap_autofill_target(
    source: CellArea,
    to_row: i32,
    to_col: i32,
    merges: &[MergedCell],
) -> AutofillTarget {
    let fill_rows = to_row < source.r1 || to_row > source.r2;
    if fill_rows {
        let down = to_row > source.r2;
        let row = snap_span(
            to_row,
            down,
            merges.iter().filter_map(|mc| {
                // Only merges inside the fill band can be cut by the fill boundary.
                if mc.column + mc.width - 1 < source.c1 || mc.column > source.c2 {
                    return None;
                }
                Some((mc.row, mc.row + mc.height - 1))
            }),
        );
        AutofillTarget {
            row,
            col: source.c1,
        }
    } else {
        let right = to_col > source.c2;
        let col = snap_span(
            to_col,
            right,
            merges.iter().filter_map(|mc| {
                if mc.row + mc.height - 1 < source.r1 || mc.row > source.r2 {
                    return None;
                }
                Some((mc.column, mc.column + mc.width - 1))
            }),
        );
        AutofillTarget {
            row: source.r1,
            col,
        }
    }
}

/// Move `target` out of any `(start, end)` span the fill boundary would cut.
///
/// Filling forward covers `start + 1..=target`, so a target anywhere in
/// `start..end` cuts the span; filling backward covers `target..=end - 1`, so a
/// target in `start..=end` cuts it. Both snap outward — to `end` forward, to
/// `start` backward — which is the extent the engine accepts (the whole merge is
/// replaced, never cut).
fn snap_span(target: i32, forward: bool, spans: impl Iterator<Item = (i32, i32)>) -> i32 {
    let mut target = target;
    for (start, end) in spans {
        let cuts = if forward {
            target >= start && target < end
        } else {
            target > start && target <= end
        };
        if cuts {
            target = if forward { end } else { start };
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merge(row: i32, column: i32, width: i32, height: i32) -> MergedCell {
        MergedCell {
            row,
            column,
            width,
            height,
        }
    }

    fn area(r1: i32, c1: i32, r2: i32, c2: i32) -> CellArea {
        CellArea { r1, c1, r2, c2 }
    }

    /// A downward drag that stops inside a merge snaps to the merge's last row,
    /// the only forward extent that does not cut it, and pins the column to the
    /// selection so the ghost cannot widen.
    #[test]
    fn a_downward_drag_inside_a_merge_snaps_to_its_last_row() {
        let merges = [merge(6, 1, 2, 3)]; // rows 6..=8, columns 1..=2
        let target = snap_autofill_target(area(1, 1, 1, 1), 7, 1, &merges);
        assert_eq!((target.row, target.col), (8, 1));
    }

    #[test]
    fn an_upward_drag_inside_a_merge_snaps_to_its_first_row() {
        let merges = [merge(1, 1, 2, 3)]; // rows 1..=3
        let target = snap_autofill_target(area(6, 1, 6, 1), 2, 1, &merges);
        assert_eq!((target.row, target.col), (1, 1));
    }

    #[test]
    fn a_rightward_drag_inside_a_merge_snaps_to_its_last_column() {
        let merges = [merge(1, 6, 3, 2)]; // columns 6..=8
        let target = snap_autofill_target(area(1, 1, 1, 1), 1, 7, &merges);
        assert_eq!((target.row, target.col), (1, 8));
    }

    /// A merge outside the fill band must not move the target: the fill cannot
    /// cut a merge it does not reach.
    #[test]
    fn a_merge_outside_the_fill_band_is_ignored() {
        let merges = [merge(6, 5, 2, 3)]; // columns 5..=6, source is column 1
        let target = snap_autofill_target(area(1, 1, 1, 1), 7, 1, &merges);
        assert_eq!((target.row, target.col), (7, 1));
    }

    #[test]
    fn a_target_on_the_far_merge_boundary_is_left_alone() {
        let merges = [merge(6, 1, 2, 3)]; // rows 6..=8
        let target = snap_autofill_target(area(1, 1, 1, 1), 8, 1, &merges);
        assert_eq!((target.row, target.col), (8, 1));
    }

    /// The near edge cuts too: filling down to the merge's *first* row covers
    /// part of it, which the engine rejects. Snapping must reach the far row.
    #[test]
    fn a_downward_drag_onto_the_near_edge_snaps_past_the_merge() {
        let merges = [merge(6, 1, 2, 3)]; // rows 6..=8
        let target = snap_autofill_target(area(1, 1, 1, 1), 6, 1, &merges);
        assert_eq!((target.row, target.col), (8, 1));
    }

    /// Upward mirror: filling up to the merge's last row cuts it, so the target
    /// snaps to the row just above the merge.
    #[test]
    fn an_upward_drag_onto_the_far_edge_snaps_before_the_merge() {
        let merges = [merge(3, 1, 2, 3)]; // rows 3..=5
        let target = snap_autofill_target(area(8, 1, 8, 1), 5, 1, &merges);
        assert_eq!((target.row, target.col), (3, 1));
    }

    /// A horizontal drag snaps the column the same way and pins the row.
    #[test]
    fn a_leftward_drag_onto_the_far_edge_snaps_before_the_merge() {
        let merges = [merge(1, 3, 2, 3)]; // columns 3..=4
        let target = snap_autofill_target(area(1, 8, 1, 8), 1, 4, &merges);
        assert_eq!((target.row, target.col), (1, 3));
    }
}
