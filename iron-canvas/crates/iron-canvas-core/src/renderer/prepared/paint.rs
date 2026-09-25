use crate::address::RCRange;
use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::frame::work::RowSpan;
use crate::geometry::prim::Axis;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cell::PaneCells;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::distinct_rows;
use crate::renderer::prepared::{FetchedCells, SegmentData};
use crate::renderer::repaint::envelope as repaint;

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

pub(super) fn paint_repaint_envelope<P: Painter>(
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

pub(super) fn paint_full_segment<P: Painter>(
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

pub(super) fn paint_segment_span<P: Painter>(
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

pub(super) fn paint_strip<P: Painter>(
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
