use crate::address::RCRange;
use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::frame::GridVerdict;
use crate::frame::work::RowSpan;
use crate::geometry::prim::Axis;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cell::PaneCells;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::DiagFingerprintActionTag;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::distinct_rows;
use crate::renderer::prepared::{
    FetchedCells, GridCacheCommit, PreparedFingerprintUpdate, PreparedGrid, PreparedRepaintPlan,
    SegmentData,
};
use crate::renderer::repaint::envelope as repaint;

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
    envelope: repaint::Envelope,
) {
    let repaint::Envelope::Visible { clip, sources } = envelope else {
        debug_assert_eq!(envelope, repaint::Envelope::NoPixels);
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
    let r1 = span.start().max(range.r1);
    let r2 = span.end().min(range.r2);
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

pub(super) fn shift_channel<E: Clone>(
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
