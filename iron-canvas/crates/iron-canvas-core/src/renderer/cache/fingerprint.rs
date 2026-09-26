//! Exact-layout grid fingerprints: the retained-pixel truth the grid cache
//! keeps across frames.
//!
//! A fingerprint stores one digest per absolute model row. Each digest folds
//! every dense column segment present for that row, so frozen-column splits do
//! not create independent truth. The frozen-row band is stored first and the
//! scroll-row band second; `scroll_band_start` makes vertical rotation explicit.
//!
//! Cache tier, next to [`GridCache`](super::GridCache), which owns one
//! [`FingerprintState`]. Comparing two trees and selecting a repaint is a paint
//! decision and lives in [`crate::renderer::repaint::plan`].

use std::cell::{Cell, Ref, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::color::data_bar_rgb;
use super::layout_transition::GridLayoutTransition;
use crate::address::RCRange;
use crate::chrome::{GridLayout, PaneRegion};
use crate::geometry::prim::Axis;
use crate::geometry::prim::Side;
use crate::model::fetched::Fetched;
use crate::renderer::prepared::FetchedCells;
use crate::style::{BorderItem, CellDecoration, CellKind, CellStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CellFingerprint(u64);

impl CellFingerprint {
    /// Flip the low digest bit, so a planner test can build two trees that
    /// differ in exactly one addressed cell.
    #[cfg(test)]
    pub(crate) fn flip(&mut self) {
        self.0 ^= 1;
    }
}

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

impl GridFingerprint {
    /// The flat leaf slice one row's `cell_start` / `cell_len` addresses.
    pub(crate) fn row_cells(&self, row: &RowFingerprint) -> Option<&[CellFingerprint]> {
        let start = row.cell_start as usize;
        let end = start.checked_add(row.cell_len as usize)?;
        self.cells.get(start..end)
    }
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

    /// Borrow exact painted history for repaint planning. A strip commit can
    /// leave the stored tree stale, so that tree must not suppress repaint.
    pub(crate) fn painted(&self) -> Option<Ref<'_, GridFingerprint>> {
        if self.truth.get() != FingerprintTruth::Exact {
            return None;
        }
        Ref::filter_map(self.painted.borrow(), Option::as_ref).ok()
    }
}

pub(crate) struct StripFingerprintSource<'a> {
    pub(crate) region: PaneRegion,
    pub(crate) range: RCRange,
    pub(crate) cells: &'a FetchedCells,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowShiftIneligible {
    StaleHistory,
    PriorLayoutMismatch,
    IncompleteStripOrExtent,
}

/// The row band a frozen or scroll half of the layout addresses. Callers use
/// it to keep a border-risk check inside the band that holds the changed rows.
pub(crate) fn band_rows(layout: GridLayout, frozen: bool) -> Option<std::ops::RangeInclusive<i32>> {
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
                    let painted_leaves = painted
                        .row_cells(painted_row)
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
            data_bar_rgb(spec).hash(hasher);
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
    use super::*;
    use crate::renderer::cache::test_support::{build, dense, layout};
    use crate::style::{Border, BorderStyle};

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
    fn stale_tree_is_unavailable_until_exact_history_is_installed() {
        let state = FingerprintState::default();
        let tree = build(layout(10, 8, 2, 2));
        assert!(state.painted().is_none());
        state.install(tree.clone());
        assert_eq!(state.painted().as_deref(), Some(&tree));
        state.mark_stale();
        assert!(state.painted().is_none());
        state.install(tree.clone());
        assert_eq!(state.painted().as_deref(), Some(&tree));
        state.reset();
        assert!(state.painted().is_none());
    }
}
