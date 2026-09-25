//! Candidate-derived address work for a pixel-only scroll-blit plan.

use crate::address::RCRange;
use crate::chrome::{Chrome, GridLayout, PaneRegion};
use crate::frame::BlitPlan;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::geometry::slot::AxisSlot;

pub(crate) struct FinalizedBlitWork {
    pub(crate) address_strips: [Option<(PaneRegion, RCRange)>; 2],
    pub(crate) pixel_clip: PixelRect,
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
            for row in region.rows(frame) {
                if row.end() > pixel_clip.top() && row.start() < pixel_clip.bottom() {
                    range.r1 = range.r1.min(row.id());
                    range.r2 = range.r2.max(row.id());
                }
            }
        }
        Axis::Column => {
            for col in region.cols(frame) {
                if col.end() > pixel_clip.left() && col.start() < pixel_clip.right() {
                    range.c1 = range.c1.min(col.id());
                    range.c2 = range.c2.max(col.id());
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
            previous.segment(region).map(|segment| segment.range()),
            candidate.segment(region).map(|segment| segment.range()),
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
