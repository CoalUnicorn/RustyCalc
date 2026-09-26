use crate::chrome::Chrome;
use crate::frame::GridVerdict;
use crate::painter::Painter;
use crate::renderer::RendererCore;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::DiagFingerprintActionTag;
use crate::renderer::prepared::{
    GridCacheCommit, PreparedFingerprintUpdate, PreparedGrid, PreparedRepaintPlan,
};

use super::paint::{paint_full_segment, paint_repaint_envelope, paint_segment_span, paint_strip};

impl<P: Painter> RendererCore<P> {
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
                match &repaint.plan {
                    PreparedRepaintPlan::Cell { envelope }
                    | PreparedRepaintPlan::Range { envelope } => {
                        paint_repaint_envelope(self, frame, layout, &mut segments, *envelope);
                    }
                    PreparedRepaintPlan::Skip => {}
                    PreparedRepaintPlan::Rows(spans) => {
                        for grid_segment in layout.segments() {
                            let data = segments[grid_segment.region().index()]
                                .as_mut()
                                .expect("every layout segment must have prepared data");
                            for span in spans {
                                paint_segment_span(self, frame, data, *span);
                            }
                        }
                    }
                    PreparedRepaintPlan::Full => {
                        for grid_segment in layout.segments() {
                            let data = segments[grid_segment.region().index()]
                                .as_mut()
                                .expect("every layout segment must have prepared data");
                            paint_full_segment(self, frame, data);
                        }
                    }
                }
                self.trace_grid(GridVerdict::from(&repaint.plan));
                #[cfg(feature = "dev-diagnostics")]
                self.diag_commit_replace(&repaint, layout);
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
                self.diag_commit_splice(&strips);
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
                self.diag_commit_shift(&address_strips, &fingerprint);
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

    pub(crate) fn commit_grid_cache(&self, commit: GridCacheCommit) {
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
