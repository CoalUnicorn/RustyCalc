//! Candidate-derived address work for a pixel-only scroll-blit plan.

use crate::chrome::{BlitPlan, Chrome, GridLayout, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::types::coord::RCRange;

pub(crate) struct FinalizedBlitWork {
    pub(crate) address_strips: [Option<(PaneRegion, RCRange)>; 2],
    pub(crate) pixel_clip: PixelRect,
}

fn segment_range(layout: GridLayout, region: PaneRegion) -> Option<RCRange> {
    layout
        .segments()
        .find(|segment| segment.region() == region)
        .map(|segment| segment.range())
}

fn revealed_strip(previous: RCRange, candidate: RCRange, axis: Axis) -> Option<RCRange> {
    match axis {
        Axis::Row if candidate.r1 < previous.r1 => Some(RCRange {
            r1: candidate.r1,
            c1: candidate.c1,
            r2: previous.r1 - 1,
            c2: candidate.c2,
        }),
        Axis::Row if candidate.r2 > previous.r2 => Some(RCRange {
            r1: previous.r2,
            c1: candidate.c1,
            r2: candidate.r2,
            c2: candidate.c2,
        }),
        Axis::Column if candidate.c1 < previous.c1 => Some(RCRange {
            r1: candidate.r1,
            c1: candidate.c1,
            r2: candidate.r2,
            c2: previous.c1 - 1,
        }),
        Axis::Column if candidate.c2 > previous.c2 => Some(RCRange {
            r1: candidate.r1,
            c1: previous.c2,
            r2: candidate.r2,
            c2: candidate.c2,
        }),
        Axis::Row | Axis::Column => None,
    }
}

fn widen_to_pixel_clip(
    frame: &Chrome,
    region: PaneRegion,
    axis: Axis,
    pixel_clip: PixelRect,
    mut range: RCRange,
) -> RCRange {
    match axis {
        Axis::Row => {
            let min = pixel_clip.top();
            let max = min + pixel_clip.height;
            for row in region.rows(frame) {
                if row.top + row.height > min && row.top < max {
                    range.r1 = range.r1.min(row.row);
                    range.r2 = range.r2.max(row.row);
                }
            }
        }
        Axis::Column => {
            let min = pixel_clip.left();
            let max = min + pixel_clip.width;
            for col in region.cols(frame) {
                if col.left + col.width > min && col.left < max {
                    range.c1 = range.c1.min(col.col);
                    range.c2 = range.c2.max(col.col);
                }
            }
        }
    }
    range
}

/// Finalize the one or two dense address strips only after the reversible
/// candidate `Chrome` exists. The classifier remains pixel-only.
pub(crate) fn finalize_blit_work(
    previous: GridLayout,
    candidate: GridLayout,
    frame: &Chrome,
    plan: &BlitPlan,
) -> Option<FinalizedBlitWork> {
    let regions = match plan.axis {
        Axis::Row => [PaneRegion::BottomLeft, PaneRegion::BottomRight],
        Axis::Column => [PaneRegion::TopRight, PaneRegion::BottomRight],
    };
    let mut address_strips = [None, None];
    for (index, region) in regions.into_iter().enumerate() {
        let (Some(previous), Some(candidate)) = (
            segment_range(previous, region),
            segment_range(candidate, region),
        ) else {
            continue;
        };
        let strip = revealed_strip(previous, candidate, plan.axis)?;
        address_strips[index] = Some((
            region,
            widen_to_pixel_clip(frame, region, plan.axis, plan.pixel_strip, strip),
        ));
    }
    if address_strips.iter().all(Option::is_none) {
        return None;
    }
    Some(FinalizedBlitWork {
        address_strips,
        pixel_clip: plan.pixel_strip,
    })
}
