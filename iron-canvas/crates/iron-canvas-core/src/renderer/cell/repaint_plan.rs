//! Grid-wide repaint planning: compare the candidate fingerprint tree with the
//! painted one and select the smallest safe repaint.
//!
//! Cell tier, next to [`repaint::Envelope`](super::repaint), which turns the
//! selected plan into executable clip geometry. The trees this module compares
//! are cache truth (`renderer/cache/fingerprint.rs`); the module reads them and
//! never mutates them.

use crate::chrome::GridLayout;
use crate::frame::work::{MAX_DAMAGE_SPANS, RowSpan};
use crate::renderer::cache::fingerprint::{GridFingerprint, band_rows};
use crate::renderer::prepared::FetchedCells;
use crate::types::coord::RCRange;

/// Grid-wide repaint decision for an exact-layout comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepaintPlan {
    Skip,
    Cell(RCRange),
    Range(RCRange),
    Rows(Vec<RowSpan>),
    Full,
}

/// The branch `plan_grid_repaint` actually took. Recorded at the decision
/// site; never re-derived by diagnostics. Only meaningful when the comparison
/// ran — Fresh-built geometry and Damage/Blit strips never produce one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepaintReason {
    NoPaintedHistory,
    LayoutMismatch,
    RowAddressMismatch,
    FingerprintsEqual,
    ChangedCell,
    ChangedCells,
    ChangedRows,
    ClipAlignment,
}

/// One grid-wide repaint decision plus the reason for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepaintDecision {
    pub(crate) plan: RepaintPlan,
    pub(crate) reason: RepaintReason,
    pub(crate) changed_rows: Vec<RowSpan>,
    pub(crate) changed_cells: Vec<RCRange>,
}

/// Compare `candidate` against the painted tree and select a repaint.
///
/// `painted` is `None` on a cold cache, after a column shift, and after a
/// damage strip: with no comparable history the only safe plan is `Full`.
pub(crate) fn plan_grid_repaint(
    painted: Option<&GridFingerprint>,
    candidate: &GridFingerprint,
) -> RepaintDecision {
    let Some(painted) = painted else {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::NoPaintedHistory,
            changed_rows: Vec::new(),
            changed_cells: Vec::new(),
        };
    };

    if painted.layout != candidate.layout
        || painted.rows.len() != candidate.rows.len()
        || painted.cells.len() != candidate.cells.len()
    {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::LayoutMismatch,
            changed_rows: Vec::new(),
            changed_cells: Vec::new(),
        };
    }

    let mut spans = Vec::<RowSpan>::new();
    for (painted_row, candidate_row) in painted.rows.iter().zip(&candidate.rows) {
        if painted_row.row != candidate_row.row {
            return RepaintDecision {
                plan: RepaintPlan::Full,
                reason: RepaintReason::RowAddressMismatch,
                changed_rows: Vec::new(),
                changed_cells: Vec::new(),
            };
        }
        if painted_row.digest == candidate_row.digest {
            continue;
        }
        if let Some(last) = spans.last_mut()
            && last.end() + 1 == candidate_row.row
        {
            *last = RowSpan::new(last.start(), candidate_row.row);
        } else {
            spans.push(RowSpan::new(candidate_row.row, candidate_row.row));
        }
    }
    if spans.is_empty() {
        return RepaintDecision {
            plan: RepaintPlan::Skip,
            reason: RepaintReason::FingerprintsEqual,
            changed_rows: Vec::new(),
            changed_cells: Vec::new(),
        };
    }

    let Ok(changed_cells) = exact_changed_cells(painted, candidate) else {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::LayoutMismatch,
            changed_rows: spans,
            changed_cells: Vec::new(),
        };
    };
    if changed_cells.len() == 1 {
        return RepaintDecision {
            plan: RepaintPlan::Cell(changed_cells[0]),
            reason: RepaintReason::ChangedCell,
            changed_rows: spans,
            changed_cells,
        };
    }
    let Some(changed_range) = bounding_range(&changed_cells) else {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::LayoutMismatch,
            changed_rows: spans,
            changed_cells,
        };
    };
    let envelope_cost = addressed_cost(candidate.layout, &[], Some(changed_range.grow_by(1)));
    let full_cost = candidate
        .layout
        .segments()
        .map(|segment| FetchedCells::addressed_cells(segment.range()))
        .sum::<usize>();
    if envelope_cost >= full_cost {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::ChangedCells,
            changed_rows: spans,
            changed_cells,
        };
    }

    let rows_are_safe = !changed_row_boundaries_have_border(painted, candidate, &spans);
    let rows_are_eligible = spans.len() <= MAX_DAMAGE_SPANS;
    let rows_cost = addressed_cost(candidate.layout, &spans, None);
    let (plan, reason) = if rows_are_safe && rows_are_eligible && rows_cost <= envelope_cost {
        (RepaintPlan::Rows(spans.clone()), RepaintReason::ChangedRows)
    } else {
        (
            RepaintPlan::Range(changed_range),
            RepaintReason::ChangedCells,
        )
    };

    RepaintDecision {
        plan,
        reason,
        changed_rows: spans,
        changed_cells,
    }
}

fn exact_changed_cells(
    painted: &GridFingerprint,
    candidate: &GridFingerprint,
) -> Result<Vec<RCRange>, ()> {
    let mut changed = Vec::new();
    for (painted_row, candidate_row) in painted.rows.iter().zip(&candidate.rows) {
        if painted_row.row != candidate_row.row {
            return Err(());
        }
        let painted_cells = painted.row_cells(painted_row).ok_or(())?;
        let candidate_cells = candidate.row_cells(candidate_row).ok_or(())?;
        if painted_cells.len() != candidate_cells.len() {
            return Err(());
        }
        if painted_row.digest == candidate_row.digest {
            continue;
        }

        let mut leaf_index = 0usize;
        for segment in candidate
            .layout
            .segments()
            .filter(|segment| segment.range().rows().contains(&candidate_row.row))
        {
            for column in segment.range().columns() {
                if painted_cells.get(leaf_index) != candidate_cells.get(leaf_index) {
                    changed.push(RCRange::from_cell(candidate_row.row, column));
                }
                leaf_index += 1;
            }
        }
        if leaf_index != candidate_cells.len() {
            return Err(());
        }
    }
    Ok(changed)
}

fn bounding_range(cells: &[RCRange]) -> Option<RCRange> {
    cells
        .iter()
        .copied()
        .map(RCRange::normalized)
        .reduce(|a, b| RCRange {
            r1: a.r1.min(b.r1),
            c1: a.c1.min(b.c1),
            r2: a.r2.max(b.r2),
            c2: a.c2.max(b.c2),
        })
}

fn range_intersection(a: RCRange, b: RCRange) -> Option<RCRange> {
    let a = a.normalized();
    let b = b.normalized();
    let intersection = RCRange {
        r1: a.r1.max(b.r1),
        c1: a.c1.max(b.c1),
        r2: a.r2.min(b.r2),
        c2: a.c2.min(b.c2),
    };
    (intersection.r1 <= intersection.r2 && intersection.c1 <= intersection.c2)
        .then_some(intersection)
}

fn addressed_cost(layout: GridLayout, spans: &[RowSpan], range: Option<RCRange>) -> usize {
    layout
        .segments()
        .map(|segment| {
            let segment_range = segment.range();
            if let Some(range) = range {
                return range_intersection(segment_range, range)
                    .map(FetchedCells::addressed_cells)
                    .unwrap_or(0);
            }
            spans
                .iter()
                .map(|span| {
                    range_intersection(
                        segment_range,
                        RCRange {
                            r1: span.start(),
                            c1: segment_range.c1,
                            r2: span.end(),
                            c2: segment_range.c2,
                        },
                    )
                    .map(FetchedCells::addressed_cells)
                    .unwrap_or(0)
                })
                .sum()
        })
        .sum()
}

fn changed_row_boundaries_have_border(
    painted: &GridFingerprint,
    candidate: &GridFingerprint,
    spans: &[RowSpan],
) -> bool {
    spans.iter().any(|span| {
        [true, false].into_iter().any(|frozen| {
            let Some(band) = band_rows(candidate.layout, frozen) else {
                return false;
            };
            let band_start = *band.start();
            let band_end = *band.end();
            let start = span.start().max(band_start);
            let end = span.end().min(band_end);
            start <= end
                && ((start > band_start
                    && rows_have_border(painted, candidate, [start - 1, start]))
                    || (end < band_end && rows_have_border(painted, candidate, [end, end + 1])))
        })
    })
}

fn rows_have_border(
    painted: &GridFingerprint,
    candidate: &GridFingerprint,
    rows: [i32; 2],
) -> bool {
    [painted, candidate].into_iter().any(|tree| {
        rows.into_iter().any(|row| {
            tree.rows
                .iter()
                .find(|tree_row| tree_row.row == row)
                .is_some_and(|fingerprint| fingerprint.has_any_explicit_border)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::cache::test_support::{build, layout};

    #[test]
    fn repaint_plans_skip_cell_and_full() {
        let exact_layout = layout(10, 8, 2, 2);
        let painted = build(exact_layout);
        let decision = plan_grid_repaint(Some(&painted), &painted);
        assert_eq!(decision.plan, RepaintPlan::Skip);
        assert_eq!(decision.reason, RepaintReason::FingerprintsEqual);

        let mut changed = painted.clone();
        let row = changed.scroll_band_start + 1;
        let cell = changed.rows[row].cell_start as usize;
        changed.cells[cell].flip();
        changed.rows[row].digest ^= 1;
        let decision = plan_grid_repaint(Some(&painted), &changed);
        assert_eq!(decision.plan, RepaintPlan::Cell(decision.changed_cells[0]));
        assert_eq!(decision.reason, RepaintReason::ChangedCell);
        assert_eq!(decision.changed_cells.len(), 1);

        let mut unsafe_change = changed.clone();
        unsafe_change.rows[row].has_any_explicit_border = true;
        let decision = plan_grid_repaint(Some(&painted), &unsafe_change);
        assert!(matches!(decision.plan, RepaintPlan::Cell(_)));
        assert_eq!(decision.reason, RepaintReason::ChangedCell);

        let shifted = build(layout(11, 8, 2, 2));
        let decision = plan_grid_repaint(Some(&painted), &shifted);
        assert_eq!(decision.plan, RepaintPlan::Full);
        assert_eq!(decision.reason, RepaintReason::LayoutMismatch);
    }

    #[test]
    fn plan_without_painted_history_is_full() {
        let decision = plan_grid_repaint(None, &build(layout(10, 8, 2, 2)));
        assert_eq!(decision.plan, RepaintPlan::Full);
        assert_eq!(decision.reason, RepaintReason::NoPaintedHistory);
        assert!(decision.changed_rows.is_empty());
        assert!(decision.changed_cells.is_empty());
    }
}
