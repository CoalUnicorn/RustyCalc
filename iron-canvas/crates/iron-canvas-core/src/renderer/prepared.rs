//! Prepared grid work: fallible model reads are completed before painter or
//! committed cache state is touched.

use crate::CellContentQuery;
use crate::chrome::{BlitPlan, Chrome, GridLayout, GridSegment, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::orchestrator::GridVerdict;
use crate::painter::{PaintColor, Painter};
use crate::pending_work::RowSpan;
use crate::renderer::RendererCore;
use crate::renderer::blit_work;
use crate::renderer::cache::BufferTruth;
use crate::renderer::cell::PaneCells;
use crate::renderer::cell::fingerprint::{
    GridFingerprint, GridLayoutTransition, RepaintPlan, RepaintReason, RowShiftIneligible,
    StripFingerprintSource,
};
use crate::renderer::cell::repaint::{CellRepaintEnvelope, build_cell_repaint_envelope};
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::{
    DiagBlitResultTag, DiagCacheActionTag, DiagFetchPurpose, DiagFingerprintActionTag,
    distinct_rows,
};
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

#[derive(Default, Clone)]
pub(crate) struct FetchedCells {
    styles: Vec<Fetched<CellStyle>>,
    values: Vec<Fetched<String>>,
    cell_types: Vec<Fetched<CellKind>>,
    decorations: Vec<Fetched<CellDecoration>>,
}

impl FetchedCells {
    pub(crate) const CHANNEL_COUNT: usize = 4;

    pub(crate) fn addressed_cells(range: RCRange) -> usize {
        let rows = (range.r2 - range.r1 + 1).max(0) as usize;
        let columns = (range.c2 - range.c1 + 1).max(0) as usize;
        rows.saturating_mul(columns)
    }

    pub(crate) fn logical_channel_slots(range: RCRange) -> usize {
        Self::addressed_cells(range).saturating_mul(Self::CHANNEL_COUNT)
    }

    pub(crate) fn is_dense_for(&self, range: RCRange) -> bool {
        let expected = Self::addressed_cells(range);
        self.styles.len() == expected
            && self.values.len() == expected
            && self.cell_types.len() == expected
            && self.decorations.len() == expected
    }

    #[cfg(any(test, feature = "surface-introspection"))]
    pub(super) fn capacities(&self) -> (usize, usize, usize, usize) {
        (
            self.styles.capacity(),
            self.values.capacity(),
            self.cell_types.capacity(),
            self.decorations.capacity(),
        )
    }

    #[cfg(test)]
    pub(crate) fn from_parts(
        styles: Vec<Fetched<CellStyle>>,
        values: Vec<Fetched<String>>,
        cell_types: Vec<Fetched<CellKind>>,
        decorations: Vec<Fetched<CellDecoration>>,
    ) -> Self {
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    pub(crate) fn styles(&self) -> &[Fetched<CellStyle>] {
        &self.styles
    }

    pub(crate) fn values(&self) -> &[Fetched<String>] {
        &self.values
    }

    pub(crate) fn cell_types(&self) -> &[Fetched<CellKind>] {
        &self.cell_types
    }

    pub(crate) fn decorations(&self) -> &[Fetched<CellDecoration>] {
        &self.decorations
    }

    pub(crate) fn fetch_into(
        model: &dyn CellContentQuery,
        sheet: u32,
        range: RCRange,
        reuse: Self,
    ) -> Self {
        let Self {
            mut styles,
            mut values,
            mut cell_types,
            mut decorations,
        } = reuse;
        model.get_cell_styles_in(sheet, range, &mut styles);
        model.get_formatted_cell_values_in(sheet, range, &mut values);
        model.get_cell_types_in(sheet, range, &mut cell_types);
        model.get_cell_decorations_in(sheet, range, &mut decorations);
        Self {
            styles,
            values,
            cell_types,
            decorations,
        }
    }

    pub(super) fn as_mut(&mut self) -> FetchedCellsMut<'_> {
        debug_assert!(
            self.styles.len() == self.values.len()
                && self.styles.len() == self.cell_types.len()
                && self.styles.len() == self.decorations.len()
        );
        FetchedCellsMut {
            styles: &mut self.styles,
            values: &mut self.values,
            cell_types: &mut self.cell_types,
            decorations: &mut self.decorations,
        }
    }

    pub(super) fn splice_strip_from(
        &mut self,
        strip: &mut Self,
        segment_range: RCRange,
        strip_range: RCRange,
    ) {
        super::cell::splice_strip_into(
            &mut self.styles,
            &mut strip.styles,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.values,
            &mut strip.values,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.cell_types,
            &mut strip.cell_types,
            segment_range,
            strip_range,
        );
        super::cell::splice_strip_into(
            &mut self.decorations,
            &mut strip.decorations,
            segment_range,
            strip_range,
        );
    }

    pub(crate) fn has_bridge_failure(&self) -> bool {
        super::cell::has_bridge_failure(&self.styles)
            || super::cell::has_bridge_failure(&self.values)
            || super::cell::has_bridge_failure(&self.cell_types)
            || super::cell::has_bridge_failure(&self.decorations)
    }

    fn shift(&mut self, previous: RCRange, candidate: RCRange, axis: Axis) {
        shift_channel(&mut self.styles, previous, candidate, axis, Fetched::Absent);
        shift_channel(&mut self.values, previous, candidate, axis, Fetched::Absent);
        shift_channel(
            &mut self.cell_types,
            previous,
            candidate,
            axis,
            Fetched::Absent,
        );
        shift_channel(
            &mut self.decorations,
            previous,
            candidate,
            axis,
            Fetched::Absent,
        );
    }
}

pub(super) struct FetchedCellsMut<'a> {
    pub(super) styles: &'a mut [Fetched<CellStyle>],
    pub(super) values: &'a mut [Fetched<String>],
    pub(super) cell_types: &'a mut [Fetched<CellKind>],
    pub(super) decorations: &'a mut [Fetched<CellDecoration>],
}

pub(crate) enum PreparedFingerprintUpdate {
    Install(GridFingerprint),
    MarkStale,
}

pub(crate) struct PreparedRepaint {
    pub(crate) plan: RepaintPlan,
    pub(crate) envelope: Option<CellRepaintEnvelope>,
    pub(crate) candidate: GridFingerprint,
    /// `Some` only when the fingerprint comparison ran. Fresh-built
    /// geometry repaints `Full` without a comparison — its authority is
    /// the attempt's `RebuildReason`, not a fingerprint branch. Read only
    /// by the dev-diagnostics recorder; unread in feature-off builds.
    #[cfg_attr(not(feature = "dev-diagnostics"), allow(dead_code))]
    pub(crate) reason: Option<RepaintReason>,
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) changed_rows: Vec<RowSpan>,
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) changed_cells: Vec<RCRange>,
}

pub(crate) struct SegmentData {
    pub(crate) segment: GridSegment,
    pub(crate) fetched: FetchedCells,
}

pub(crate) struct PreparedStrip {
    pub(crate) region: PaneRegion,
    pub(crate) range: RCRange,
    pub(crate) fetched: FetchedCells,
}

// Full preparation uses fixed segment storage by design. Boxing the large arm
// would trade this predictable stack value for a heap allocation per frame.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PreparedGrid {
    Empty,
    Full {
        layout: GridLayout,
        segments: [Option<SegmentData>; 4],
        repaint: PreparedRepaint,
    },
    Damage {
        layout: GridLayout,
        strips: Vec<PreparedStrip>,
    },
    Blit {
        previous: GridLayout,
        layout: GridLayout,
        axis: Axis,
        address_strips: [Option<PreparedStrip>; 2],
        pixel_clip: PixelRect,
        fingerprint: PreparedFingerprintUpdate,
    },
}

pub(crate) enum GridCacheCommit {
    Replace {
        layout: GridLayout,
        segments: [Option<FetchedCells>; 4],
        fingerprint: GridFingerprint,
    },
    Shift {
        previous: GridLayout,
        layout: GridLayout,
        axis: Axis,
        address_strips: [Option<PreparedStrip>; 2],
        fingerprint: PreparedFingerprintUpdate,
    },
    Splice {
        layout: GridLayout,
        strips: Vec<PreparedStrip>,
        fingerprint: PreparedFingerprintUpdate,
    },
    Reset,
}

impl<P: Painter> RendererCore<P> {
    fn take_strip_scratch(&self) -> FetchedCells {
        self.frame_cache
            .strip_scratch
            .borrow_mut()
            .pop()
            .unwrap_or_default()
    }

    fn park_strip_scratch(&self, cells: FetchedCells) {
        self.frame_cache.strip_scratch.borrow_mut().push(cells);
    }

    pub(crate) fn prepare_full_grid(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
    ) -> Option<PreparedGrid> {
        let layout = frame.grid_layout();
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, layout);
        let mut segments: [Option<SegmentData>; 4] = std::array::from_fn(|_| None);
        for grid_segment in layout.segments() {
            let region = grid_segment.region();
            let range = grid_segment.range();
            let scratch = self.grid_cache.take_prepare_scratch(region);
            let fetched = FetchedCells::fetch_into(model, frame.sheet, range, scratch);
            self.trace_fetch(range);
            #[cfg(feature = "dev-diagnostics")]
            self.diag_fetch(DiagFetchPurpose::FullSegment, Some(region), range);
            if fetched.has_bridge_failure() {
                self.grid_cache.park_prepare_scratch(region, fetched);
                for prepared in segments.into_iter().flatten() {
                    self.grid_cache
                        .park_prepare_scratch(prepared.segment.region(), prepared.fetched);
                }
                self.trace_frame_held();
                return None;
            }
            segments[region.index()] = Some(SegmentData {
                segment: grid_segment,
                fetched,
            });
        }

        if layout.segments().next().is_none() {
            #[cfg(feature = "dev-diagnostics")]
            self.diag_cache_planned(DiagCacheActionTag::Reset);
            for prepared in segments.into_iter().flatten() {
                self.grid_cache
                    .park_prepare_scratch(prepared.segment.region(), prepared.fetched);
            }
            return Some(PreparedGrid::Empty);
        }

        let fetched: [Option<&FetchedCells>; 4] =
            std::array::from_fn(|index| segments[index].as_ref().map(|segment| &segment.fetched));
        let candidate = self
            .grid_cache
            .fingerprint
            .build_candidate(layout, &fetched);
        let (mut plan, mut reason, changed_rows, changed_cells) = if frame.kind.reuses_slots() {
            let decision = self.grid_cache.fingerprint.compare_to_painted(&candidate);
            (
                decision.plan,
                Some(decision.reason),
                decision.changed_rows,
                decision.changed_cells,
            )
        } else {
            (RepaintPlan::Full, None, Vec::new(), Vec::new())
        };
        let envelope = if matches!(plan, RepaintPlan::Cell(_) | RepaintPlan::Range(_)) {
            match build_cell_repaint_envelope(frame, &changed_cells) {
                CellRepaintEnvelope::UnalignedDpr => {
                    plan = RepaintPlan::Full;
                    reason = Some(RepaintReason::ClipAlignment);
                    None
                }
                envelope => Some(envelope),
            }
        } else {
            None
        };
        #[cfg(not(feature = "dev-diagnostics"))]
        {
            drop(changed_rows);
            drop(changed_cells);
        }
        #[cfg(feature = "dev-diagnostics")]
        self.diag_cache_planned(DiagCacheActionTag::Replace);
        Some(PreparedGrid::Full {
            layout,
            segments,
            repaint: PreparedRepaint {
                plan,
                envelope,
                candidate,
                reason,
                #[cfg(feature = "dev-diagnostics")]
                changed_rows,
                #[cfg(feature = "dev-diagnostics")]
                changed_cells,
            },
        })
    }

    pub(crate) fn prepare_damage_grid(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> Option<PreparedGrid> {
        let layout = frame.grid_layout();
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, layout);
        if self.grid_cache.layout() != Some(layout)
            || self.grid_cache.buffer_truth() != BufferTruth::Valid
        {
            return self.prepare_full_grid(model, frame);
        }

        let mut strips: Vec<PreparedStrip> = Vec::new();
        for grid_segment in layout.segments() {
            let range = grid_segment.range();
            for span in spans {
                let r1 = span.r1.max(range.r1);
                let r2 = span.r2.min(range.r2);
                if r1 > r2 {
                    continue;
                }
                let strip_range = RCRange {
                    r1,
                    c1: range.c1,
                    r2,
                    c2: range.c2,
                };
                let fetched = FetchedCells::fetch_into(
                    model,
                    frame.sheet,
                    strip_range,
                    self.take_strip_scratch(),
                );
                self.trace_fetch(strip_range);
                #[cfg(feature = "dev-diagnostics")]
                self.diag_fetch(
                    DiagFetchPurpose::DamageStrip,
                    Some(grid_segment.region()),
                    strip_range,
                );
                if fetched.has_bridge_failure() {
                    self.park_strip_scratch(fetched);
                    for strip in strips {
                        self.park_strip_scratch(strip.fetched);
                    }
                    self.trace_frame_held();
                    return None;
                }
                strips.push(PreparedStrip {
                    region: grid_segment.region(),
                    range: strip_range,
                    fetched,
                });
            }
        }
        #[cfg(feature = "dev-diagnostics")]
        self.diag_cache_planned(DiagCacheActionTag::Splice);
        Some(PreparedGrid::Damage { layout, strips })
    }

    pub(crate) fn prepare_blit_grid(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> Option<PreparedGrid> {
        let candidate = frame.grid_layout();
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, candidate);
        let transition = self.grid_cache.classify_layout(candidate);
        let GridLayoutTransition::Shift { axis } = transition else {
            self.trace_blit_fallback(self.grid_cache.layout().is_none());
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(self.grid_cache.layout().is_none()),
                self.grid_cache.layout(),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        };
        let same_axis = matches!(
            (axis, plan.axis),
            (Axis::Row, Axis::Row) | (Axis::Column, Axis::Column)
        );
        if !same_axis || self.grid_cache.buffer_truth() != BufferTruth::Valid {
            self.trace_blit_fallback(self.grid_cache.layout().is_none());
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(self.grid_cache.layout().is_none()),
                self.grid_cache.layout(),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        }
        let previous = self
            .grid_cache
            .layout()
            .expect("a Shift transition always has a committed layout");
        let Some(work) = blit_work::finalize_blit_work(previous, candidate, frame, plan) else {
            // A classified Shift should always expose at least one address
            // strip. If geometry rules drift, repainting the candidate is a
            // safe recovery; treating this as a bridge hold would retry the
            // same impossible plan indefinitely.
            self.trace_blit_fallback(false);
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(false),
                Some(previous),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        };
        let mut address_strips: [Option<PreparedStrip>; 2] = std::array::from_fn(|_| None);
        for (index, strip) in work.address_strips.into_iter().enumerate() {
            let Some((region, range)) = strip else {
                continue;
            };
            let fetched = FetchedCells::fetch_into(
                model,
                frame.sheet,
                range,
                self.grid_cache.take_prepare_scratch(region),
            );
            self.trace_fetch(range);
            #[cfg(feature = "dev-diagnostics")]
            self.diag_fetch(DiagFetchPurpose::BlitReveal, Some(region), range);
            if fetched.has_bridge_failure() {
                self.grid_cache.park_prepare_scratch(region, fetched);
                for prepared in address_strips.into_iter().flatten() {
                    self.grid_cache
                        .park_prepare_scratch(prepared.region, prepared.fetched);
                }
                self.trace_frame_held();
                #[cfg(feature = "dev-diagnostics")]
                self.diag_blit(
                    plan,
                    DiagBlitResultTag::HeldPreflight,
                    None,
                    Some(previous),
                    candidate,
                );
                return None;
            }
            address_strips[index] = Some(PreparedStrip {
                region,
                range,
                fetched,
            });
        }

        let fingerprint = if matches!(axis, Axis::Row) {
            let sources: Vec<_> = address_strips
                .iter()
                .flatten()
                .map(|strip| StripFingerprintSource {
                    region: strip.region,
                    range: strip.range,
                    cells: &strip.fetched,
                })
                .collect();
            match self
                .grid_cache
                .fingerprint
                .build_row_shift_candidate(previous, candidate, &sources)
            {
                Ok(candidate) => PreparedFingerprintUpdate::Install(candidate),
                Err(
                    RowShiftIneligible::StaleHistory
                    | RowShiftIneligible::PriorLayoutMismatch
                    | RowShiftIneligible::IncompleteStripOrExtent,
                ) => PreparedFingerprintUpdate::MarkStale,
            }
        } else {
            PreparedFingerprintUpdate::MarkStale
        };
        #[cfg(feature = "dev-diagnostics")]
        {
            self.diag_blit(
                plan,
                DiagBlitResultTag::Shifted,
                None,
                Some(previous),
                candidate,
            );
            for strip in address_strips.iter().flatten() {
                self.diag_blit_revealed(strip.region, strip.range);
            }
            self.diag_cache_planned(DiagCacheActionTag::Shift);
        }
        Some(PreparedGrid::Blit {
            previous,
            layout: candidate,
            axis,
            address_strips,
            pixel_clip: work.pixel_clip,
            fingerprint,
        })
    }

    pub(crate) fn execute_prepared_grid(
        &self,
        frame: &Chrome,
        prepared: PreparedGrid,
    ) -> GridCacheCommit {
        match prepared {
            PreparedGrid::Empty => {
                #[cfg(feature = "dev-diagnostics")]
                self.diag_fingerprint_action(DiagFingerprintActionTag::Reset);
                GridCacheCommit::Reset
            }
            PreparedGrid::Full {
                layout,
                mut segments,
                repaint,
            } => {
                let _painted_envelope = match &repaint.plan {
                    RepaintPlan::Cell(_) | RepaintPlan::Range(_) => {
                        paint_repaint_envelope(
                            self,
                            frame,
                            layout,
                            &mut segments,
                            repaint
                                .envelope
                                .expect("an envelope plan must carry prepared geometry"),
                        );
                        true
                    }
                    RepaintPlan::Skip | RepaintPlan::Rows(_) | RepaintPlan::Full => {
                        for grid_segment in layout.segments() {
                            let data = segments[grid_segment.region().index()]
                                .as_mut()
                                .expect("every layout segment must have prepared data");
                            match &repaint.plan {
                                RepaintPlan::Skip => {}
                                RepaintPlan::Rows(spans) => {
                                    for span in spans {
                                        paint_segment_span(self, frame, data, *span);
                                    }
                                }
                                RepaintPlan::Full => paint_full_segment(self, frame, data),
                                RepaintPlan::Cell(_) | RepaintPlan::Range(_) => {
                                    unreachable!("envelope plans execute before the segment loop")
                                }
                            }
                        }
                        false
                    }
                };
                self.trace_grid(GridVerdict::from(&repaint.plan));
                #[cfg(feature = "dev-diagnostics")]
                {
                    let verdict = GridVerdict::from(&repaint.plan);
                    self.diag_repaint(
                        verdict,
                        repaint.reason,
                        &repaint.changed_rows,
                        &repaint.changed_cells,
                    );
                }
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(DiagFingerprintActionTag::Install);
                    // Absolute row intervals per painted segment, merged so
                    // `rows` counts distinct grid rows even when frozen
                    // columns visit the same rows in left and right
                    // segments. Cells stay disjoint across segments.
                    if !_painted_envelope {
                        let mut row_intervals: Vec<(i32, i32)> = Vec::new();
                        let mut cells = 0usize;
                        for grid_segment in layout.segments() {
                            let range = grid_segment.range();
                            let cols = (range.c2 - range.c1 + 1).max(0) as usize;
                            match &repaint.plan {
                                RepaintPlan::Skip => {}
                                RepaintPlan::Full => {
                                    row_intervals.push((range.r1, range.r2));
                                    cells += FetchedCells::addressed_cells(range);
                                }
                                RepaintPlan::Rows(spans) => {
                                    for span in spans {
                                        let r1 = span.r1.max(range.r1);
                                        let r2 = span.r2.min(range.r2);
                                        if r1 <= r2 {
                                            row_intervals.push((r1, r2));
                                            cells += (r2 - r1 + 1) as usize * cols;
                                        }
                                    }
                                }
                                RepaintPlan::Cell(_) | RepaintPlan::Range(_) => {
                                    unreachable!("envelope counts were returned by execution")
                                }
                            }
                        }
                        self.diag_paint_counts(distinct_rows(&row_intervals), cells);
                    }
                }
                GridCacheCommit::Replace {
                    layout,
                    segments: std::array::from_fn(|index| {
                        segments[index].take().map(|segment| segment.fetched)
                    }),
                    fingerprint: repaint.candidate,
                }
            }
            PreparedGrid::Damage { layout, mut strips } => {
                for strip in &mut strips {
                    paint_strip(self, frame, strip.region, strip.range, &mut strip.fetched);
                }
                self.trace_grid(GridVerdict::Strip);
                #[cfg(feature = "dev-diagnostics")]
                self.diag_repaint(GridVerdict::Strip, None, &[], &[]);
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(DiagFingerprintActionTag::MarkStale);
                    let row_intervals: Vec<(i32, i32)> = strips
                        .iter()
                        .map(|strip| (strip.range.r1, strip.range.r2))
                        .collect();
                    let cells = strips
                        .iter()
                        .map(|strip| FetchedCells::addressed_cells(strip.range))
                        .sum();
                    self.diag_paint_counts(distinct_rows(&row_intervals), cells);
                }
                GridCacheCommit::Splice {
                    layout,
                    strips,
                    fingerprint: PreparedFingerprintUpdate::MarkStale,
                }
            }
            PreparedGrid::Blit {
                previous,
                layout,
                axis,
                mut address_strips,
                pixel_clip,
                fingerprint,
            } => {
                self.painter.push_clip(pixel_clip);
                #[cfg(feature = "dev-diagnostics")]
                self.diag_blit_clip(pixel_clip);
                for strip in address_strips.iter_mut().flatten() {
                    paint_strip(self, frame, strip.region, strip.range, &mut strip.fetched);
                }
                self.painter.pop_clip();
                self.trace_grid(GridVerdict::Strip);
                #[cfg(feature = "dev-diagnostics")]
                self.diag_repaint(GridVerdict::Strip, None, &[], &[]);
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(match &fingerprint {
                        PreparedFingerprintUpdate::Install(_) => DiagFingerprintActionTag::Install,
                        PreparedFingerprintUpdate::MarkStale => DiagFingerprintActionTag::MarkStale,
                    });
                    let row_intervals: Vec<(i32, i32)> = address_strips
                        .iter()
                        .flatten()
                        .map(|strip| (strip.range.r1, strip.range.r2))
                        .collect();
                    let cells = address_strips
                        .iter()
                        .flatten()
                        .map(|strip| FetchedCells::addressed_cells(strip.range))
                        .sum();
                    self.diag_paint_counts(distinct_rows(&row_intervals), cells);
                }
                GridCacheCommit::Shift {
                    previous,
                    layout,
                    axis,
                    address_strips,
                    fingerprint,
                }
            }
        }
    }

    pub(super) fn commit_grid_cache(&self, commit: GridCacheCommit) {
        match commit {
            GridCacheCommit::Replace {
                layout,
                segments,
                fingerprint,
            } => {
                self.grid_cache.replace_cells(layout, segments);
                self.grid_cache.fingerprint.install(fingerprint);
            }
            GridCacheCommit::Shift {
                previous,
                layout,
                axis,
                mut address_strips,
                fingerprint,
            } => {
                let mut cells = self.grid_cache.take_cells();
                for grid_segment in layout.segments() {
                    let region = grid_segment.region();
                    let previous_range = previous
                        .segment(region)
                        .expect("a compatible Shift preserves segment presence")
                        .range();
                    cells[region.index()]
                        .as_mut()
                        .expect("valid grid buffers contain every shifted segment")
                        .shift(previous_range, grid_segment.range(), axis);
                }
                for strip in address_strips.iter_mut().flatten() {
                    let segment_range = layout
                        .segment(strip.region)
                        .expect("a committed blit strip belongs to the candidate layout")
                        .range();
                    cells[strip.region.index()]
                        .as_mut()
                        .expect("valid grid buffers contain every shifted segment")
                        .splice_strip_from(&mut strip.fetched, segment_range, strip.range);
                }
                self.grid_cache.restore_cells(layout, cells);
                for strip in address_strips.into_iter().flatten() {
                    self.grid_cache
                        .park_prepare_scratch(strip.region, strip.fetched);
                }
                match fingerprint {
                    PreparedFingerprintUpdate::Install(candidate) => {
                        self.grid_cache.fingerprint.install(candidate)
                    }
                    PreparedFingerprintUpdate::MarkStale => {
                        self.grid_cache.fingerprint.mark_stale()
                    }
                }
            }
            GridCacheCommit::Splice {
                layout,
                mut strips,
                fingerprint,
            } => {
                let mut cells = self.grid_cache.take_cells();
                for strip in &mut strips {
                    let segment_range = layout
                        .segment(strip.region)
                        .expect("a committed Damage strip belongs to the exact layout")
                        .range();
                    cells[strip.region.index()]
                        .as_mut()
                        .expect("valid grid buffers contain every layout segment")
                        .splice_strip_from(&mut strip.fetched, segment_range, strip.range);
                }
                self.grid_cache.restore_cells(layout, cells);
                for strip in strips {
                    self.park_strip_scratch(strip.fetched);
                }
                match fingerprint {
                    PreparedFingerprintUpdate::Install(candidate) => {
                        self.grid_cache.fingerprint.install(candidate)
                    }
                    PreparedFingerprintUpdate::MarkStale => {
                        self.grid_cache.fingerprint.mark_stale()
                    }
                }
            }
            GridCacheCommit::Reset => self.grid_cache.reset(),
        }
    }
}

/// Binds the cells visited by a paint pass to the address range that owns
/// the dense fetched-buffer indexing. These ranges intentionally differ for
/// row-span and repaint-envelope walks over full-segment fetches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaintCellsRanges {
    walk_range: RCRange,
    index_range: RCRange,
}

fn paint_cells_in<P: Painter>(
    renderer: &RendererCore<P>,
    frame: &Chrome,
    region: PaneRegion,
    fetched: &mut FetchedCells,
    ranges: PaintCellsRanges,
) {
    debug_assert!(
        ranges
            .index_range
            .contains(ranges.walk_range.r1, ranges.walk_range.c1)
            && ranges
                .index_range
                .contains(ranges.walk_range.r2, ranges.walk_range.c2),
        "paint walk must stay inside the fetched-buffer index range"
    );
    debug_assert!(
        fetched.is_dense_for(ranges.index_range),
        "fetched buffer must be dense for its index range"
    );
    renderer.paint_cells_pass(
        PaneCells::for_strip(&region, frame, ranges.walk_range),
        ranges.index_range,
        &frame.theme,
        fetched.as_mut(),
    );
}

fn paint_repaint_envelope<P: Painter>(
    renderer: &RendererCore<P>,
    frame: &Chrome,
    layout: GridLayout,
    segments: &mut [Option<SegmentData>; 4],
    envelope: CellRepaintEnvelope,
) {
    let CellRepaintEnvelope::Visible { clip, sources } = envelope else {
        debug_assert_eq!(envelope, CellRepaintEnvelope::NoPixels);
        #[cfg(feature = "dev-diagnostics")]
        {
            renderer.diag_repaint_envelope(None, &[None; 4]);
            renderer.diag_paint_counts(0, 0);
        }
        return;
    };

    renderer.painter.push_clip(clip);
    #[cfg(feature = "dev-diagnostics")]
    renderer.diag_repaint_envelope(Some(clip), &sources);
    renderer
        .painter
        .rect_fill(clip, PaintColor::from_theme_str(&frame.theme.cell_bg));
    #[cfg(feature = "dev-diagnostics")]
    let mut row_intervals = Vec::new();
    #[cfg(feature = "dev-diagnostics")]
    let mut painted_cells = 0usize;
    for grid_segment in layout.segments() {
        let region = grid_segment.region();
        let Some(source) = sources[region.index()] else {
            continue;
        };
        let data = segments[region.index()]
            .as_mut()
            .expect("every contributor source belongs to a prepared segment");
        paint_cells_in(
            renderer,
            frame,
            region,
            &mut data.fetched,
            PaintCellsRanges {
                walk_range: source,
                index_range: data.segment.range(),
            },
        );
        #[cfg(feature = "dev-diagnostics")]
        {
            row_intervals.push((source.r1, source.r2));
            painted_cells += FetchedCells::addressed_cells(source);
        }
    }
    renderer.painter.pop_clip();

    #[cfg(feature = "dev-diagnostics")]
    renderer.diag_paint_counts(distinct_rows(&row_intervals), painted_cells);
}

fn paint_full_segment<P: Painter>(
    renderer: &RendererCore<P>,
    frame: &Chrome,
    data: &mut SegmentData,
) {
    let range = data.segment.range();
    let region = data.segment.region();
    if frame.kind.reuses_slots()
        && let Some(rect) = frame.range_rect(range)
    {
        renderer
            .painter
            .rect_fill(rect, PaintColor::from_theme_str(&frame.theme.cell_bg));
    }
    paint_cells_in(
        renderer,
        frame,
        region,
        &mut data.fetched,
        PaintCellsRanges {
            walk_range: range,
            index_range: range,
        },
    );
}

fn paint_segment_span<P: Painter>(
    renderer: &RendererCore<P>,
    frame: &Chrome,
    data: &mut SegmentData,
    span: RowSpan,
) {
    let range = data.segment.range();
    let r1 = span.r1.max(range.r1);
    let r2 = span.r2.min(range.r2);
    if r1 > r2 {
        return;
    }
    let strip = RCRange {
        r1,
        c1: range.c1,
        r2,
        c2: range.c2,
    };
    if let Some(rect) = frame.range_rect(strip) {
        renderer
            .painter
            .rect_fill(rect, PaintColor::from_theme_str(&frame.theme.cell_bg));
    }
    paint_cells_in(
        renderer,
        frame,
        data.segment.region(),
        &mut data.fetched,
        PaintCellsRanges {
            walk_range: strip,
            index_range: range,
        },
    );
}

fn paint_strip<P: Painter>(
    renderer: &RendererCore<P>,
    frame: &Chrome,
    region: PaneRegion,
    strip_range: RCRange,
    cells: &mut FetchedCells,
) {
    if let Some(rect) = frame.range_rect(strip_range) {
        renderer
            .painter
            .rect_fill(rect, PaintColor::from_theme_str(&frame.theme.cell_bg));
    }
    paint_cells_in(
        renderer,
        frame,
        region,
        cells,
        PaintCellsRanges {
            walk_range: strip_range,
            index_range: strip_range,
        },
    );
}

fn shift_channel<E: Clone>(
    channel: &mut [E],
    previous: RCRange,
    candidate: RCRange,
    axis: Axis,
    fill: E,
) {
    let rows = (previous.r2 - previous.r1 + 1).max(0) as usize;
    let cols = (previous.c2 - previous.c1 + 1).max(0) as usize;
    debug_assert_eq!(channel.len(), rows * cols);
    match axis {
        Axis::Row => {
            let delta = candidate.r1 - previous.r1;
            if delta > 0 {
                let shift = delta as usize * cols;
                channel.rotate_left(shift);
                let len = channel.len();
                channel[len - shift..].fill(fill);
            } else if delta < 0 {
                let shift = (-delta) as usize * cols;
                channel.rotate_right(shift);
                channel[..shift].fill(fill);
            }
        }
        Axis::Column => {
            let delta = candidate.c1 - previous.c1;
            for row in channel.chunks_exact_mut(cols) {
                if delta > 0 {
                    let shift = delta as usize;
                    row.rotate_left(shift);
                    row[cols - shift..].fill(fill.clone());
                } else if delta < 0 {
                    let shift = (-delta) as usize;
                    row.rotate_right(shift);
                    row[..shift].fill(fill.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_failure_is_detected_in_each_dense_channel() {
        fn clean<T>() -> Vec<Fetched<T>> {
            vec![Fetched::Absent]
        }
        fn failed<T>() -> Vec<Fetched<T>> {
            vec![Fetched::BridgeFailed]
        }

        let range = RCRange::from_cell(1, 1);
        for cells in [
            FetchedCells::from_parts(failed(), clean(), clean(), clean()),
            FetchedCells::from_parts(clean(), failed(), clean(), clean()),
            FetchedCells::from_parts(clean(), clean(), failed(), clean()),
            FetchedCells::from_parts(clean(), clean(), clean(), failed()),
        ] {
            assert!(cells.is_dense_for(range));
            assert!(cells.has_bridge_failure());
        }
    }
}
