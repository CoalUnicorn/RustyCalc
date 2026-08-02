//! Renderer-facing blit pane work.
//!
//! [`PaneBlitAddressWork`](crate::renderer::cache::PaneBlitAddressWork) is the
//! cache's address-only half (computed in `renderer/cache/`, no `Chrome`
//! dependency). This module widens it against the frame's slot geometry into a
//! [`BlitPaneWork`] the renderer consumes: the address-space `strip_range`
//! grows to cover every visible slot whose pixel extent overlaps the plan's
//! repaint strip, and a `pixel_clip` is attached only for the main scroll pane.
//!
//! `BlitPaneWork` is transient — built per shifted pane each blit frame, never
//! stored on `PaneBuffers` or `Chrome`.

use crate::chrome::{BlitPlan, Chrome, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::renderer::cache::{PaneBlitAddressWork, PaneCache, PaneShiftPrep};
use crate::types::coord::RCRange;

/// One shifted pane's complete blit work for the renderer: the address-space
/// strip to fetch/paint (widened to the pixel clip) plus an optional pixel
/// clip applied while painting.
pub struct BlitPaneWork {
    pub pane: PaneRegion,
    pub prev_range: RCRange,
    pub new_range: RCRange,
    pub axis: Axis,
    pub strip_range: RCRange,
    /// Clip applied while painting this pane's strip. `Some` only for the main
    /// scroll pane that is clipped today (`BottomRight`); frozen-band sibling
    /// panes (`BottomLeft` on row scroll, `TopRight` on column scroll) paint
    /// their narrowed `strip_range` without an extra clip.
    pub pixel_clip: Option<PixelRect>,
}

/// Widen a pane's address-space strip to every visible slot whose pixel extent
/// overlaps `plan.repaint_strip`, and attach the pixel clip for the main pane.
///
/// `compute_strip` (in `renderer/cache/`) is an address-space proxy for the
/// pixel repaint strip; the two agree only when slot edges land on the canvas
/// edge. On a non-aligned axis the partial slot at the canvas boundary
/// transitions to fully-visible inside the dirty pixel rect — this loop extends
/// the `RCRange` to cover it. Only the main `BottomRight` pane carries the
/// `pixel_clip`; frozen-band siblings paint their narrowed range unclipped.
pub fn widen_blit_strip_to_pixel_clip(
    frame: &Chrome,
    plan: &BlitPlan,
    pane: PaneRegion,
    work: PaneBlitAddressWork,
) -> BlitPaneWork {
    let PaneBlitAddressWork {
        axis,
        prev_range,
        new_range,
        mut strip_range,
    } = work;

    let repaint_strip = plan.repaint_strip;
    match axis {
        Axis::Column => {
            let xmin = repaint_strip.top_left.x;
            let xmax = xmin + repaint_strip.width;
            let mut new_c1 = strip_range.c1;
            let mut new_c2 = strip_range.c2;
            for c in pane.cols(frame) {
                if c.left + c.width > xmin && c.left < xmax {
                    new_c1 = new_c1.min(c.col);
                    new_c2 = new_c2.max(c.col);
                }
            }
            strip_range.c1 = new_c1;
            strip_range.c2 = new_c2;
        }
        Axis::Row => {
            let ymin = repaint_strip.top_left.y;
            let ymax = ymin + repaint_strip.height;
            let mut new_r1 = strip_range.r1;
            let mut new_r2 = strip_range.r2;
            for r in pane.rows(frame) {
                if r.top + r.height > ymin && r.top < ymax {
                    new_r1 = new_r1.min(r.row);
                    new_r2 = new_r2.max(r.row);
                }
            }
            strip_range.r1 = new_r1;
            strip_range.r2 = new_r2;
        }
    }

    let pixel_clip = match pane {
        PaneRegion::BottomRight => Some(plan.repaint_strip),
        PaneRegion::TopLeft | PaneRegion::TopRight | PaneRegion::BottomLeft => None,
    };

    BlitPaneWork {
        pane,
        prev_range,
        new_range,
        axis,
        strip_range,
        pixel_clip,
    }
}

/// Classify `pane`'s cached range against `plan`'s live geometry and, only on
/// a genuine `Shifted` result, widen it into the pane's complete
/// [`BlitPaneWork`] — the one call [`crate::renderer::prepared::RendererCore::prepare_blit`]
/// makes per `plan.shift_panes()` pane to decide the strip-vs-full-pane fork.
///
/// `None` covers every reason this pane cannot strip-paint this frame: never
/// cached, an incompatible cached range (e.g. a canvas resize since the last
/// fetch), or the defensive zero-delta/non-overlap guard inside
/// [`PaneCache::plan_blit_pane`]. The caller routes all three uniformly to a
/// full-pane fallback fetch without needing to know which one fired.
///
/// Pure: reads `pane_cache`/`frame`/`plan` only, via
/// [`crate::renderer::cache::PaneBuffers::classify_shift`] — never rotates
/// the pane's buffers or mutates its cached range. The actual rotation is
/// execution's job, once the revealed strip's fetch is confirmed clean (see
/// `renderer::prepared`'s module doc).
pub(crate) fn shifted_pane_work(
    pane_cache: &PaneCache,
    frame: &Chrome,
    plan: &BlitPlan,
    pane: PaneRegion,
) -> Option<BlitPaneWork> {
    let new_range = pane.range(frame)?;
    let PaneShiftPrep::Shifted {
        prev_range,
        new_range,
    } = pane_cache.pane(pane).classify_shift(new_range, plan.axis)
    else {
        return None;
    };
    let address_work = pane_cache.plan_blit_pane(prev_range, new_range, plan.axis)?;
    Some(widen_blit_strip_to_pixel_clip(
        frame,
        plan,
        pane,
        address_work,
    ))
}
