//! Exact-layout grid fingerprints used for retained-pixel repaint planning.
//!
//! A fingerprint stores one digest per absolute model row. Each digest folds
//! every dense column segment present for that row, so frozen-column splits do
//! not create independent truth. The frozen-row band is stored first and the
//! scroll-row band second; `scroll_band_start` makes vertical rotation explicit.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::chrome::{GridLayout, PaneRegion};
use crate::geometry::prim::Axis;
use crate::orchestrator::GridVerdict;
use crate::pending_work::{MAX_DAMAGE_SPANS, RowSpan};
use crate::renderer::cf_types::parse_hex_color;
use crate::renderer::prepared::FetchedCells;
use crate::style::{BorderItem, CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;
use crate::types::ui::Side;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellFingerprint(u64);

/// One grid row's paint digest, shared-border risk, and flat leaf slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowFingerprint {
    pub(crate) row: i32,
    pub(crate) digest: u64,
    pub(crate) has_any_explicit_border: bool,
    pub(crate) cell_start: u32,
    pub(crate) cell_len: u32,
}

/// Fingerprint truth keyed by the complete address layout that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridFingerprint {
    pub(crate) layout: GridLayout,
    pub(crate) rows: Vec<RowFingerprint>,
    pub(crate) cells: Vec<CellFingerprint>,
    pub(crate) scroll_band_start: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum FingerprintTruth {
    Exact,
    #[default]
    Stale,
}

/// The last fingerprint known to describe painted pixels plus a reusable
/// allocation slot for the next candidate.
#[derive(Default)]
pub(crate) struct FingerprintState {
    painted: RefCell<Option<GridFingerprint>>,
    scratch: RefCell<Option<GridFingerprint>>,
    truth: Cell<FingerprintTruth>,
}

impl FingerprintState {
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn truth(&self) -> FingerprintTruth {
        self.truth.get()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum GridLayoutTransition {
    Exact,
    Shift { axis: Axis },
    Incompatible,
}

impl GridLayoutTransition {
    /// Classify only exact-layout compatibility. Buffer validity is an
    /// independent grid-cache fact and must not change this result.
    pub(crate) fn classify(committed: GridLayout, candidate: GridLayout) -> Self {
        if committed == candidate {
            return Self::Exact;
        }
        if committed.shape() != candidate.shape() {
            return Self::Incompatible;
        }

        let unchanged = |region| committed.segment(region) == candidate.segment(region);
        let shifted = |region: PaneRegion, axis: Axis| {
            let before = committed.segment(region);
            let after = candidate.segment(region);
            match (before, after) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    let before = before.range();
                    let after = after.range();
                    match axis {
                        Axis::Row => {
                            before.c1 == after.c1
                                && before.c2 == after.c2
                                && before.r2 - before.r1 == after.r2 - after.r1
                                && before.r1 != after.r1
                        }
                        Axis::Column => {
                            before.r1 == after.r1
                                && before.r2 == after.r2
                                && before.c2 - before.c1 == after.c2 - after.c1
                                && before.c1 != after.c1
                        }
                    }
                }
                (None, Some(_)) | (Some(_), None) => false,
            }
        };

        if unchanged(PaneRegion::TopLeft)
            && unchanged(PaneRegion::TopRight)
            && shifted(PaneRegion::BottomLeft, Axis::Row)
            && shifted(PaneRegion::BottomRight, Axis::Row)
        {
            return Self::Shift { axis: Axis::Row };
        }

        if unchanged(PaneRegion::TopLeft)
            && unchanged(PaneRegion::BottomLeft)
            && shifted(PaneRegion::TopRight, Axis::Column)
            && shifted(PaneRegion::BottomRight, Axis::Column)
        {
            return Self::Shift { axis: Axis::Column };
        }

        Self::Incompatible
    }
}

pub(crate) struct StripFingerprintSource<'a> {
    pub(crate) region: PaneRegion,
    pub(crate) range: RCRange,
    pub(crate) cells: &'a FetchedCells,
}

fn band_rows(layout: GridLayout, frozen: bool) -> Option<std::ops::RangeInclusive<i32>> {
    let regions = if frozen {
        [PaneRegion::TopLeft, PaneRegion::TopRight]
    } else {
        [PaneRegion::BottomLeft, PaneRegion::BottomRight]
    };
    regions
        .into_iter()
        .find_map(|region| layout.segment(region).map(|segment| segment.range().rows()))
}

fn fingerprint_grid_row(
    layout: GridLayout,
    row: i32,
    cells: &[Option<&FetchedCells>; 4],
    leaves: &mut Vec<CellFingerprint>,
) -> RowFingerprint {
    let cell_start = leaves.len();
    let mut row_hasher = DefaultHasher::new();
    row_hasher.write_i32(row);
    let mut has_any_explicit_border = false;

    for grid_segment in layout
        .segments()
        .filter(|segment| segment.range().rows().contains(&row))
    {
        let range = grid_segment.range();
        let fetched = cells[grid_segment.region().index()]
            .expect("every layout segment must have a fetched bundle");
        debug_assert!(fetched.is_dense_for(range));
        let cols = (range.c2 - range.c1 + 1).max(0) as usize;
        let base = (row - range.r1).max(0) as usize * cols;
        for (col_offset, col) in range.columns().enumerate() {
            let idx = base + col_offset;
            let style = &fetched.styles()[idx];
            has_any_explicit_border |= style_has_explicit_border(style);
            let digest = cell_digest(
                row,
                col,
                style,
                &fetched.values()[idx],
                &fetched.cell_types()[idx],
                &fetched.decorations()[idx],
            );
            leaves.push(CellFingerprint(digest));
            digest.hash(&mut row_hasher);
        }
    }

    RowFingerprint {
        row,
        digest: row_hasher.finish(),
        has_any_explicit_border,
        cell_start: cell_start as u32,
        cell_len: (leaves.len() - cell_start) as u32,
    }
}

fn fingerprint_strip_row(
    layout: GridLayout,
    row: i32,
    strips: &[StripFingerprintSource<'_>],
    leaves: &mut Vec<CellFingerprint>,
) -> Option<RowFingerprint> {
    let cell_start = leaves.len();
    let mut row_hasher = DefaultHasher::new();
    row_hasher.write_i32(row);
    let mut has_any_explicit_border = false;
    let mut found_segment = false;

    for grid_segment in layout
        .segments()
        .filter(|segment| segment.range().rows().contains(&row))
    {
        found_segment = true;
        let segment_range = grid_segment.range();
        let Some(strip) = strips.iter().find(|strip| {
            strip.region == grid_segment.region()
                && strip.range.rows().contains(&row)
                && strip.range.c1 == segment_range.c1
                && strip.range.c2 == segment_range.c2
                && strip.cells.is_dense_for(strip.range)
        }) else {
            leaves.truncate(cell_start);
            return None;
        };
        let cols = (strip.range.c2 - strip.range.c1 + 1).max(0) as usize;
        let base = (row - strip.range.r1).max(0) as usize * cols;
        for (col_offset, col) in strip.range.columns().enumerate() {
            let idx = base + col_offset;
            let style = &strip.cells.styles()[idx];
            has_any_explicit_border |= style_has_explicit_border(style);
            let digest = cell_digest(
                row,
                col,
                style,
                &strip.cells.values()[idx],
                &strip.cells.cell_types()[idx],
                &strip.cells.decorations()[idx],
            );
            leaves.push(CellFingerprint(digest));
            digest.hash(&mut row_hasher);
        }
    }

    found_segment.then(|| RowFingerprint {
        row,
        digest: row_hasher.finish(),
        has_any_explicit_border,
        cell_start: cell_start as u32,
        cell_len: (leaves.len() - cell_start) as u32,
    })
}

fn style_has_explicit_border(style: &Fetched<CellStyle>) -> bool {
    let Fetched::Value(style) = style else {
        return false;
    };
    Side::ALL
        .into_iter()
        .any(|side| style.border.get(side).is_some())
}

fn rebuild_grid_fingerprint(
    target: &mut GridFingerprint,
    layout: GridLayout,
    cells: &[Option<&FetchedCells>; 4],
) {
    target.layout = layout;
    target.rows.clear();
    target.cells.clear();
    if let Some(rows) = band_rows(layout, true) {
        for row in rows {
            target
                .rows
                .push(fingerprint_grid_row(layout, row, cells, &mut target.cells));
        }
    }
    target.scroll_band_start = target.rows.len();
    if let Some(rows) = band_rows(layout, false) {
        for row in rows {
            target
                .rows
                .push(fingerprint_grid_row(layout, row, cells, &mut target.cells));
        }
    }
}

fn empty_grid_fingerprint(layout: GridLayout) -> GridFingerprint {
    GridFingerprint {
        layout,
        rows: Vec::new(),
        cells: Vec::new(),
        scroll_band_start: 0,
    }
}

impl FingerprintState {
    pub(crate) fn build_candidate(
        &self,
        layout: GridLayout,
        cells: &[Option<&FetchedCells>; 4],
    ) -> GridFingerprint {
        let mut candidate = self
            .scratch
            .borrow_mut()
            .take()
            .unwrap_or_else(|| empty_grid_fingerprint(layout));
        rebuild_grid_fingerprint(&mut candidate, layout, cells);
        candidate
    }

    pub(crate) fn compare_to_painted(&self, candidate: &GridFingerprint) -> RepaintDecision {
        self.painted.borrow().as_ref().map_or_else(
            || RepaintDecision {
                plan: RepaintPlan::Full,
                reason: RepaintReason::NoPaintedHistory,
                changed_rows: Vec::new(),
                changed_cells: Vec::new(),
            },
            |painted| plan_grid_repaint(painted, candidate),
        )
    }

    pub(crate) fn install(&self, candidate: GridFingerprint) {
        let old = self.painted.borrow_mut().replace(candidate);
        *self.scratch.borrow_mut() = old;
        self.truth.set(FingerprintTruth::Exact);
    }

    pub(crate) fn mark_stale(&self) {
        self.truth.set(FingerprintTruth::Stale);
    }

    pub(crate) fn reset(&self) {
        *self.painted.borrow_mut() = None;
        *self.scratch.borrow_mut() = None;
        self.truth.set(FingerprintTruth::Stale);
    }

    /// Rotate only the scroll-row band. Frozen rows retain their original
    /// digests, overlapping scroll rows reuse painted truth, and every newly
    /// addressed row must be complete across all candidate column segments.
    pub(crate) fn build_row_shift_candidate(
        &self,
        previous_layout: GridLayout,
        candidate_layout: GridLayout,
        strips: &[StripFingerprintSource<'_>],
    ) -> Result<GridFingerprint, RowShiftIneligible> {
        if self.truth.get() != FingerprintTruth::Exact {
            return Err(RowShiftIneligible::StaleHistory);
        }
        let painted = self.painted.borrow();
        let Some(painted) = painted.as_ref() else {
            return Err(RowShiftIneligible::StaleHistory);
        };
        if painted.layout != previous_layout {
            return Err(RowShiftIneligible::PriorLayoutMismatch);
        }
        if !matches!(
            GridLayoutTransition::classify(previous_layout, candidate_layout),
            GridLayoutTransition::Shift { axis: Axis::Row }
        ) {
            return Err(RowShiftIneligible::IncompleteStripOrExtent);
        }

        let mut candidate = self
            .scratch
            .borrow_mut()
            .take()
            .unwrap_or_else(|| empty_grid_fingerprint(candidate_layout));
        candidate.layout = candidate_layout;
        candidate.rows.clear();
        candidate.cells.clear();

        let append_band = |frozen: bool,
                           output_rows: &mut Vec<RowFingerprint>,
                           output_cells: &mut Vec<CellFingerprint>| {
            if let Some(band) = band_rows(candidate_layout, frozen) {
                for row in band {
                    if let Some(fingerprint) =
                        fingerprint_strip_row(candidate_layout, row, strips, output_cells)
                    {
                        output_rows.push(fingerprint);
                        continue;
                    }
                    let painted_row = painted
                        .rows
                        .iter()
                        .find(|painted_row| painted_row.row == row)
                        .ok_or(RowShiftIneligible::IncompleteStripOrExtent)?;
                    let painted_leaves = row_cells(painted, painted_row)
                        .ok_or(RowShiftIneligible::IncompleteStripOrExtent)?;
                    let cell_start = output_cells.len();
                    output_cells.extend_from_slice(painted_leaves);
                    let mut copied = painted_row.clone();
                    copied.cell_start = cell_start as u32;
                    output_rows.push(copied);
                }
            }
            Ok::<(), RowShiftIneligible>(())
        };

        if let Err(reason) = append_band(true, &mut candidate.rows, &mut candidate.cells) {
            *self.scratch.borrow_mut() = Some(candidate);
            return Err(reason);
        }
        candidate.scroll_band_start = candidate.rows.len();
        if let Err(reason) = append_band(false, &mut candidate.rows, &mut candidate.cells) {
            *self.scratch.borrow_mut() = Some(candidate);
            return Err(reason);
        }
        Ok(candidate)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowShiftIneligible {
    StaleHistory,
    PriorLayoutMismatch,
    IncompleteStripOrExtent,
}

/// Grid-wide repaint decision for an exact-layout comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepaintPlan {
    Skip,
    Cell(RCRange),
    Range(RCRange),
    Rows(Vec<RowSpan>),
    Full,
}

/// The branch `plan_grid_repaint` / `compare_to_painted` actually took.
/// Recorded at the decision site; never re-derived by diagnostics. Only
/// meaningful when the comparison ran — Fresh-built geometry and
/// Damage/Blit strips never produce one.
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

impl From<&RepaintPlan> for GridVerdict {
    fn from(plan: &RepaintPlan) -> Self {
        match plan {
            RepaintPlan::Skip => Self::Skip,
            RepaintPlan::Cell(_) => Self::Cell,
            RepaintPlan::Range(_) => Self::Range,
            RepaintPlan::Rows(spans) => Self::Rows {
                spans: spans.len().min(u8::MAX as usize) as u8,
                rows: spans
                    .iter()
                    .map(|span| (span.r2 - span.r1 + 1).max(0) as u32)
                    .sum::<u32>()
                    .min(u16::MAX as u32) as u16,
            },
            RepaintPlan::Full => Self::Full,
        }
    }
}

fn plan_grid_repaint(painted: &GridFingerprint, candidate: &GridFingerprint) -> RepaintDecision {
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
            && last.r2 + 1 == candidate_row.row
        {
            last.r2 = candidate_row.row;
        } else {
            spans.push(RowSpan {
                r1: candidate_row.row,
                r2: candidate_row.row,
            });
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

fn row_cells<'a>(
    fingerprint: &'a GridFingerprint,
    row: &RowFingerprint,
) -> Option<&'a [CellFingerprint]> {
    let start = row.cell_start as usize;
    let end = start.checked_add(row.cell_len as usize)?;
    fingerprint.cells.get(start..end)
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
        let painted_cells = row_cells(painted, painted_row).ok_or(())?;
        let candidate_cells = row_cells(candidate, candidate_row).ok_or(())?;
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
                            r1: span.r1,
                            c1: segment_range.c1,
                            r2: span.r2,
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
            let start = span.r1.max(band_start);
            let end = span.r2.min(band_end);
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

/// Hash exactly the cell inputs that can affect painted pixels.
fn cell_digest(
    row: i32,
    col: i32,
    style: &Fetched<CellStyle>,
    value: &Fetched<String>,
    cell_type: &Fetched<CellKind>,
    decoration: &Fetched<CellDecoration>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_i32(row);
    hasher.write_i32(col);
    match style {
        Fetched::Absent | Fetched::BridgeFailed => hasher.write_u8(0),
        Fetched::Value(style) => {
            hasher.write_u8(1);
            StyleDigest(style).hash(&mut hasher);
        }
    }
    match value {
        Fetched::Absent | Fetched::BridgeFailed => hasher.write_u8(0),
        Fetched::Value(text) => {
            hasher.write_u8(1);
            hasher.write_usize(text.len());
            hasher.write(text.as_bytes());
        }
    }
    match cell_type {
        Fetched::Absent | Fetched::BridgeFailed => hasher.write_u8(0),
        Fetched::Value(cell_type) => {
            hasher.write_u8(1);
            std::mem::discriminant(cell_type).hash(&mut hasher);
        }
    }
    hash_decoration(decoration, &mut hasher);
    hasher.finish()
}

fn hash_decoration<H: Hasher>(decoration: &Fetched<CellDecoration>, hasher: &mut H) {
    match decoration {
        Fetched::Absent | Fetched::BridgeFailed | Fetched::Value(CellDecoration::Icon(_)) => {
            hasher.write_u8(0)
        }
        Fetched::Value(CellDecoration::DataBar(spec)) => {
            hasher.write_u8(1);
            parse_hex_color(&spec.color)
                .unwrap_or([0, 0, 0])
                .hash(hasher);
            spec.fraction.clamp(0.0, 1.0).to_bits().hash(hasher);
        }
        Fetched::Value(CellDecoration::Rating(spec)) => {
            hasher.write_u8(2);
            (spec.stars as u8).hash(hasher);
            (spec.filled as u8).hash(hasher);
        }
    }
}

struct StyleDigest<'a>(&'a CellStyle);

impl Hash for StyleDigest<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let style = self.0;
        style.fill_color.hash(state);
        style.font.strike.hash(state);
        style.font.underline.hash(state);
        style.font.bold.hash(state);
        style.font.italic.hash(state);
        style.font.size.to_bits().hash(state);
        style.font.color.hash(state);
        style.font.name.hash(state);
        match &style.alignment {
            None => state.write_u8(0),
            Some(alignment) => {
                state.write_u8(1);
                std::mem::discriminant(&alignment.horizontal).hash(state);
                std::mem::discriminant(&alignment.vertical).hash(state);
                alignment.wrap_text.hash(state);
            }
        }
        for side in [Side::Left, Side::Right, Side::Top, Side::Bottom] {
            hash_border_item(style.border.get(side), state);
        }
        style.border.diagonal_up.hash(state);
        style.border.diagonal_down.hash(state);
    }
}

fn hash_border_item<H: Hasher>(border: Option<&BorderItem>, state: &mut H) {
    match border {
        None => state.write_u8(0),
        Some(border) => {
            state.write_u8(1);
            std::mem::discriminant(&border.style).hash(state);
            border.color.hash(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::FrameInputs;
    use crate::chrome::{Chrome, FramePath};
    use crate::geometry::CanvasSize;
    use crate::model_adapter::{CanvasModel, CanvasView, CellContentQuery};
    use crate::style::{Border, BorderStyle};
    use crate::theme::CanvasTheme;

    struct LayoutModel {
        top: i32,
        left: i32,
        frozen_rows: i32,
        frozen_cols: i32,
    }

    impl CellContentQuery for LayoutModel {
        fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Fetched<CellStyle> {
            Fetched::Absent
        }

        fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Fetched<CellKind> {
            Fetched::Absent
        }

        fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Fetched<String> {
            Fetched::Absent
        }
    }

    impl CanvasModel for LayoutModel {
        fn get_selected_sheet(&self) -> Option<u32> {
            Some(0)
        }

        fn get_selected_view(&self) -> Option<CanvasView> {
            Some(CanvasView {
                sheet: 0,
                row: self.top,
                column: self.left,
                selection: RCRange::from_cell(self.top, self.left),
                top_row: self.top,
                left_column: self.left,
            })
        }

        fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
            Some(self.frozen_rows)
        }

        fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
            Some(self.frozen_cols)
        }

        fn get_row_height(&self, _: u32, _: i32) -> Option<f64> {
            Some(20.0)
        }

        fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
            Some(60.0)
        }

        fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
            Some(true)
        }
    }

    fn layout(top: i32, left: i32, frozen_rows: i32, frozen_cols: i32) -> GridLayout {
        let model = LayoutModel {
            top,
            left,
            frozen_rows,
            frozen_cols,
        };
        let inputs = FrameInputs::capture(
            &model,
            CanvasSize { w: 420.0, h: 260.0 },
            1.0,
            Rc::new(CanvasTheme::light()),
            0,
        )
        .unwrap();
        Chrome::next(None, &model, &inputs, FramePath::Fresh).grid_layout()
    }

    fn dense(range: RCRange) -> FetchedCells {
        let mut styles = Vec::new();
        let mut values = Vec::new();
        let mut cell_types = Vec::new();
        let mut decorations = Vec::new();
        for row in range.rows() {
            for col in range.columns() {
                styles.push(Fetched::Value(CellStyle::default()));
                values.push(Fetched::Value(format!("{row}:{col}")));
                cell_types.push(Fetched::Value(CellKind::Text));
                decorations.push(Fetched::Absent);
            }
        }
        FetchedCells::from_parts(styles, values, cell_types, decorations)
    }

    fn bundles(layout: GridLayout) -> [Option<FetchedCells>; 4] {
        let mut bundles = std::array::from_fn(|_| None);
        for segment in layout.segments() {
            bundles[segment.region().index()] = Some(dense(segment.range()));
        }
        bundles
    }

    fn build(layout: GridLayout) -> GridFingerprint {
        let bundles = bundles(layout);
        let references = std::array::from_fn(|index| bundles[index].as_ref());
        let state = FingerprintState::default();
        state.build_candidate(layout, &references)
    }

    #[test]
    fn paint_relevant_hash_inputs_change_the_digest() {
        let plain = CellStyle::default();
        let mut bordered = plain.clone();
        bordered.border = Border {
            bottom: Some(BorderItem {
                style: BorderStyle::Thin,
                color: Some("#123456".to_string()),
            }),
            ..Border::default()
        };
        let base = cell_digest(
            1,
            1,
            &Fetched::Value(plain),
            &Fetched::Value("a".to_string()),
            &Fetched::Value(CellKind::Text),
            &Fetched::Absent,
        );
        let value_changed = cell_digest(
            1,
            1,
            &Fetched::Value(CellStyle::default()),
            &Fetched::Value("b".to_string()),
            &Fetched::Value(CellKind::Text),
            &Fetched::Absent,
        );
        let style_changed = cell_digest(
            1,
            1,
            &Fetched::Value(bordered),
            &Fetched::Value("a".to_string()),
            &Fetched::Value(CellKind::Text),
            &Fetched::Absent,
        );
        assert_ne!(base, value_changed);
        assert_ne!(base, style_changed);
    }

    #[test]
    fn layout_classifies_exact_row_column_and_incompatible() {
        let base = layout(10, 8, 2, 2);
        assert!(matches!(
            GridLayoutTransition::classify(base, base),
            GridLayoutTransition::Exact
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(11, 8, 2, 2)),
            GridLayoutTransition::Shift { axis: Axis::Row }
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(10, 9, 2, 2)),
            GridLayoutTransition::Shift { axis: Axis::Column }
        ));
        assert!(matches!(
            GridLayoutTransition::classify(base, layout(10, 8, 0, 2)),
            GridLayoutTransition::Incompatible
        ));
    }

    #[test]
    fn single_tree_rotation_frozen_boundary() {
        let previous_layout = layout(10, 8, 2, 2);
        let candidate_layout = layout(11, 8, 2, 2);
        let state = FingerprintState::default();
        state.install(build(previous_layout));

        let mut strip_cells: [Option<FetchedCells>; 2] = std::array::from_fn(|_| None);
        let mut strip_meta: [Option<(PaneRegion, RCRange)>; 2] = [None, None];
        for (index, region) in [PaneRegion::BottomLeft, PaneRegion::BottomRight]
            .into_iter()
            .enumerate()
        {
            let previous = previous_layout
                .segment(region)
                .expect("the previous test layout contains every pane segment")
                .range();
            let candidate = candidate_layout
                .segment(region)
                .expect("the candidate test layout contains every pane segment")
                .range();
            let range = RCRange {
                r1: previous.r2 + 1,
                c1: candidate.c1,
                r2: candidate.r2,
                c2: candidate.c2,
            };
            strip_cells[index] = Some(dense(range));
            strip_meta[index] = Some((region, range));
        }
        let sources: Vec<_> = strip_meta
            .iter()
            .zip(&strip_cells)
            .filter_map(|(meta, cells)| {
                let (region, range) = (*meta)?;
                Some(StripFingerprintSource {
                    region,
                    range,
                    cells: cells.as_ref().unwrap(),
                })
            })
            .collect();

        let rotated = state
            .build_row_shift_candidate(previous_layout, candidate_layout, &sources)
            .unwrap();
        let rebuilt = build(candidate_layout);
        assert_eq!(rotated, rebuilt);
        assert!(rotated.scroll_band_start > 0);
    }

    #[test]
    fn row_rotation_rejects_stale_and_incomplete_history() {
        let previous = layout(10, 8, 2, 2);
        let candidate = layout(11, 8, 2, 2);
        let state = FingerprintState::default();
        assert_eq!(
            state.build_row_shift_candidate(previous, candidate, &[]),
            Err(RowShiftIneligible::StaleHistory)
        );
        state.install(build(previous));
        assert_eq!(
            state.build_row_shift_candidate(previous, candidate, &[]),
            Err(RowShiftIneligible::IncompleteStripOrExtent)
        );
    }

    #[test]
    fn repaint_plans_skip_cell_and_full() {
        let exact_layout = layout(10, 8, 2, 2);
        let painted = build(exact_layout);
        let decision = plan_grid_repaint(&painted, &painted);
        assert_eq!(decision.plan, RepaintPlan::Skip);
        assert_eq!(decision.reason, RepaintReason::FingerprintsEqual);

        let mut changed = painted.clone();
        let row = changed.scroll_band_start + 1;
        let cell = changed.rows[row].cell_start as usize;
        changed.cells[cell].0 ^= 1;
        changed.rows[row].digest ^= 1;
        let decision = plan_grid_repaint(&painted, &changed);
        assert_eq!(decision.plan, RepaintPlan::Cell(decision.changed_cells[0]));
        assert_eq!(decision.reason, RepaintReason::ChangedCell);
        assert_eq!(decision.changed_cells.len(), 1);

        let mut unsafe_change = changed.clone();
        unsafe_change.rows[row].has_any_explicit_border = true;
        let decision = plan_grid_repaint(&painted, &unsafe_change);
        assert!(matches!(decision.plan, RepaintPlan::Cell(_)));
        assert_eq!(decision.reason, RepaintReason::ChangedCell);

        let shifted = build(layout(11, 8, 2, 2));
        let decision = plan_grid_repaint(&painted, &shifted);
        assert_eq!(decision.plan, RepaintPlan::Full);
        assert_eq!(decision.reason, RepaintReason::LayoutMismatch);
    }
}
