//! Exact-layout grid fingerprints used for retained-pixel repaint planning.
//!
//! A fingerprint stores one digest per absolute model row. Each digest folds
//! every dense column segment present for that row, so frozen-column splits do
//! not create independent truth. The frozen-row band is stored first and the
//! scroll-row band second; `scroll_band_start` makes vertical rotation explicit.

use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::chrome::{GridLayout, GridSegment, PaneRegion};
use crate::geometry::prim::Axis;
use crate::orchestrator::GridVerdict;
use crate::pending_work::RowSpan;
use crate::renderer::cf_types::parse_hex_color;
use crate::renderer::prepared::FetchedCells;
use crate::style::{BorderItem, CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// One grid row's paint digest and conservative shared-border risk bit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowDigest {
    pub(crate) digest: u64,
    pub(crate) has_any_explicit_border: bool,
}

/// Fingerprint truth keyed by the complete address layout that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GridFingerprint {
    pub(crate) layout: GridLayout,
    pub(crate) rows: Vec<(i32, RowDigest)>,
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

#[derive(Clone, Copy)]
pub(crate) enum GridLayoutTransition {
    Exact,
    Shift { axis: Axis },
    Incompatible,
}

fn segment(layout: GridLayout, region: PaneRegion) -> Option<GridSegment> {
    layout.segments().find(|segment| segment.region() == region)
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

        let unchanged = |region| segment(committed, region) == segment(candidate, region);
        let shifted = |region: PaneRegion, axis: Axis| {
            let before = segment(committed, region);
            let after = segment(candidate, region);
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
        .find_map(|region| segment(layout, region).map(|segment| segment.range().rows()))
}

fn fingerprint_grid_row(
    layout: GridLayout,
    row: i32,
    cells: &[Option<&FetchedCells>; 4],
) -> RowDigest {
    let mut row_hasher = DefaultHasher::new();
    row_hasher.write_i32(row);
    let mut has_any_explicit_border = false;

    for grid_segment in layout
        .segments()
        .filter(|segment| segment.range().rows().contains(&row))
    {
        let range = grid_segment.range();
        let fetched = cells[grid_segment.region() as usize]
            .expect("every layout segment must have a fetched bundle");
        debug_assert!(fetched.is_dense_for(range));
        let cols = (range.c2 - range.c1 + 1).max(0) as usize;
        let base = (row - range.r1).max(0) as usize * cols;
        for (col_offset, col) in range.columns().enumerate() {
            let idx = base + col_offset;
            let style = &fetched.styles()[idx];
            has_any_explicit_border |= style_has_explicit_border(style);
            cell_digest(
                row,
                col,
                style,
                &fetched.values()[idx],
                &fetched.cell_types()[idx],
                &fetched.decorations()[idx],
            )
            .hash(&mut row_hasher);
        }
    }

    RowDigest {
        digest: row_hasher.finish(),
        has_any_explicit_border,
    }
}

fn fingerprint_strip_row(
    layout: GridLayout,
    row: i32,
    strips: &[StripFingerprintSource<'_>],
) -> Option<RowDigest> {
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
        let strip = strips.iter().find(|strip| {
            strip.region == grid_segment.region()
                && strip.range.rows().contains(&row)
                && strip.range.c1 == segment_range.c1
                && strip.range.c2 == segment_range.c2
                && strip.cells.is_dense_for(strip.range)
        })?;
        let cols = (strip.range.c2 - strip.range.c1 + 1).max(0) as usize;
        let base = (row - strip.range.r1).max(0) as usize * cols;
        for (col_offset, col) in strip.range.columns().enumerate() {
            let idx = base + col_offset;
            let style = &strip.cells.styles()[idx];
            has_any_explicit_border |= style_has_explicit_border(style);
            cell_digest(
                row,
                col,
                style,
                &strip.cells.values()[idx],
                &strip.cells.cell_types()[idx],
                &strip.cells.decorations()[idx],
            )
            .hash(&mut row_hasher);
        }
    }

    found_segment.then(|| RowDigest {
        digest: row_hasher.finish(),
        has_any_explicit_border,
    })
}

fn style_has_explicit_border(style: &Fetched<CellStyle>) -> bool {
    let Fetched::Value(style) = style else {
        return false;
    };
    style.border.left.is_some()
        || style.border.right.is_some()
        || style.border.top.is_some()
        || style.border.bottom.is_some()
}

fn rebuild_grid_fingerprint(
    target: &mut GridFingerprint,
    layout: GridLayout,
    cells: &[Option<&FetchedCells>; 4],
) {
    target.layout = layout;
    target.rows.clear();
    if let Some(rows) = band_rows(layout, true) {
        target
            .rows
            .extend(rows.map(|row| (row, fingerprint_grid_row(layout, row, cells))));
    }
    target.scroll_band_start = target.rows.len();
    if let Some(rows) = band_rows(layout, false) {
        target
            .rows
            .extend(rows.map(|row| (row, fingerprint_grid_row(layout, row, cells))));
    }
}

fn empty_grid_fingerprint(layout: GridLayout) -> GridFingerprint {
    GridFingerprint {
        layout,
        rows: Vec::new(),
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

    pub(crate) fn compare_to_painted(&self, candidate: &GridFingerprint) -> RepaintPlan {
        self.painted
            .borrow()
            .as_ref()
            .map_or(RepaintPlan::Full, |painted| {
                plan_grid_repaint(painted, candidate)
            })
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

        let append_band = |frozen: bool, output: &mut Vec<(i32, RowDigest)>| {
            if let Some(band) = band_rows(candidate_layout, frozen) {
                for row in band {
                    let digest = fingerprint_strip_row(candidate_layout, row, strips)
                        .or_else(|| {
                            painted
                                .rows
                                .iter()
                                .find(|(painted_row, _)| *painted_row == row)
                                .map(|(_, digest)| digest.clone())
                        })
                        .ok_or(RowShiftIneligible::IncompleteStripOrExtent)?;
                    output.push((row, digest));
                }
            }
            Ok::<(), RowShiftIneligible>(())
        };

        if let Err(reason) = append_band(true, &mut candidate.rows) {
            *self.scratch.borrow_mut() = Some(candidate);
            return Err(reason);
        }
        candidate.scroll_band_start = candidate.rows.len();
        if let Err(reason) = append_band(false, &mut candidate.rows) {
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
    Rows(Vec<RowSpan>),
    Full,
}

impl From<&RepaintPlan> for GridVerdict {
    fn from(plan: &RepaintPlan) -> Self {
        match plan {
            RepaintPlan::Skip => Self::Skip,
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

fn plan_grid_repaint(painted: &GridFingerprint, candidate: &GridFingerprint) -> RepaintPlan {
    if painted.layout != candidate.layout || painted.rows.len() != candidate.rows.len() {
        return RepaintPlan::Full;
    }

    let mut spans = Vec::<RowSpan>::new();
    for ((painted_row, painted_digest), (candidate_row, candidate_digest)) in
        painted.rows.iter().zip(&candidate.rows)
    {
        if painted_row != candidate_row {
            return RepaintPlan::Full;
        }
        if painted_digest == candidate_digest {
            continue;
        }
        if let Some(last) = spans.last_mut()
            && last.r2 + 1 == *candidate_row
        {
            last.r2 = *candidate_row;
        } else if spans.len() < 8 {
            spans.push(RowSpan {
                r1: *candidate_row,
                r2: *candidate_row,
            });
        } else {
            return RepaintPlan::Full;
        }
    }
    if spans.is_empty() {
        return RepaintPlan::Skip;
    }

    for span in &spans {
        for frozen in [true, false] {
            let Some(band) = band_rows(candidate.layout, frozen) else {
                continue;
            };
            let band_start = *band.start();
            let band_end = *band.end();
            let start = span.r1.max(band_start);
            let end = span.r2.min(band_end);
            if start > end {
                continue;
            }
            if start > band_start && rows_have_border(painted, candidate, [start - 1, start]) {
                return RepaintPlan::Full;
            }
            if end < band_end && rows_have_border(painted, candidate, [end, end + 1]) {
                return RepaintPlan::Full;
            }
        }
    }

    RepaintPlan::Rows(spans)
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
                .find(|(tree_row, _)| *tree_row == row)
                .is_some_and(|(_, digest)| digest.has_any_explicit_border)
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
        hash_border_item(&style.border.left, state);
        hash_border_item(&style.border.right, state);
        hash_border_item(&style.border.top, state);
        hash_border_item(&style.border.bottom, state);
        style.border.diagonal_up.hash(state);
        style.border.diagonal_down.hash(state);
    }
}

fn hash_border_item<H: Hasher>(border: &Option<BorderItem>, state: &mut H) {
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
            bundles[segment.region() as usize] = Some(dense(segment.range()));
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
    fn frozen_band_row_rotation_matches_full_candidate() {
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
            let previous = segment(previous_layout, region).unwrap().range();
            let candidate = segment(candidate_layout, region).unwrap().range();
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
    fn repaint_plans_skip_rows_and_full() {
        let exact_layout = layout(10, 8, 2, 2);
        let painted = build(exact_layout);
        assert_eq!(plan_grid_repaint(&painted, &painted), RepaintPlan::Skip);

        let mut changed = painted.clone();
        let row = changed.scroll_band_start + 1;
        changed.rows[row].1.digest ^= 1;
        assert_eq!(
            plan_grid_repaint(&painted, &changed),
            RepaintPlan::Rows(vec![RowSpan {
                r1: changed.rows[row].0,
                r2: changed.rows[row].0,
            }])
        );

        let mut unsafe_change = changed.clone();
        unsafe_change.rows[row].1.has_any_explicit_border = true;
        assert_eq!(
            plan_grid_repaint(&painted, &unsafe_change),
            RepaintPlan::Full
        );

        let shifted = build(layout(11, 8, 2, 2));
        assert_eq!(plan_grid_repaint(&painted, &shifted), RepaintPlan::Full);
    }
}
