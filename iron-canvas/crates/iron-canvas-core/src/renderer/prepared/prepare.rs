use crate::CellContentQuery;
use crate::address::RCRange;
use crate::chrome::Chrome;
use crate::frame::BlitPlan;
use crate::frame::work::RowSpan;
use crate::geometry::prim::Axis;
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::renderer::blit;
use crate::renderer::cache::BufferTruth;
use crate::renderer::cache::fingerprint::{RowShiftIneligible, StripFingerprintSource};
use crate::renderer::cache::layout_transition::GridLayoutTransition;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::{DiagBlitResultTag, DiagCacheActionTag, DiagFetchPurpose};
use crate::renderer::prepared::{
    FetchedCells, PreparedFingerprintUpdate, PreparedGrid, PreparedRepaint, PreparedRepaintPlan,
    PreparedStrip, SegmentData,
};
use crate::renderer::repaint::envelope as repaint;
use crate::renderer::repaint::plan as repaint_plan;
use crate::renderer::repaint::plan::{RepaintPlan, RepaintReason};

impl<P: Painter> RendererCore<P> {
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
        let (plan, reason, changed_rows, changed_cells) = if frame.kind.reuses_slots() {
            let decision = repaint_plan::plan_grid_repaint(
                self.grid_cache.fingerprint.painted().as_deref(),
                &candidate,
            );
            let mut reason = decision.reason;
            let plan = match decision.plan {
                RepaintPlan::Cell(_) => {
                    match repaint::build_envelope(frame, &decision.changed_cells) {
                        repaint::EnvelopeBuild::Ready(envelope) => {
                            PreparedRepaintPlan::Cell { envelope }
                        }
                        repaint::EnvelopeBuild::UnalignedDpr => {
                            reason = RepaintReason::ClipAlignment;
                            PreparedRepaintPlan::Full
                        }
                    }
                }
                RepaintPlan::Range(_) => {
                    match repaint::build_envelope(frame, &decision.changed_cells) {
                        repaint::EnvelopeBuild::Ready(envelope) => {
                            PreparedRepaintPlan::Range { envelope }
                        }
                        repaint::EnvelopeBuild::UnalignedDpr => {
                            reason = RepaintReason::ClipAlignment;
                            PreparedRepaintPlan::Full
                        }
                    }
                }
                RepaintPlan::Skip => PreparedRepaintPlan::Skip,
                RepaintPlan::Rows(spans) => PreparedRepaintPlan::Rows(spans),
                RepaintPlan::Full => PreparedRepaintPlan::Full,
            };
            (
                plan,
                Some(reason),
                decision.changed_rows,
                decision.changed_cells,
            )
        } else {
            (PreparedRepaintPlan::Full, None, Vec::new(), Vec::new())
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
                let r1 = span.start().max(range.r1);
                let r2 = span.end().min(range.r2);
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
        let Some(work) = blit::finalize_blit_work(previous, candidate, frame, plan) else {
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
}
