//! Classification of a captured `FrameInputs` against the committed `Chrome`:
//! whether the next frame is a Stable reuse, a safe scroll, or a full rebuild
//! (and why).
//!
//! [`FrameDelta`] and [`RebuildReason`] are the classification types
//! [`Chrome::classify`](crate::chrome::Chrome::classify) produces. They live
//! here so the crate public re-export surface stays stable regardless of
//! which module owns the comparison logic; `Chrome::classify` is the sole
//! producer. `FrameDelta::Scroll` carries its [`BlitPlan`] payload, so this
//! module imports nothing from `chrome` and the old `chrome` <-> `frame`
//! cycle is gone.
//!
//! [`BlitPlan`] is constructed by `Chrome::prepare_blit` / `next_blit`, whose
//! `impl` block lives in `chrome/blit.rs`; only the data type lives here.

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;

/// Outcome of classifying a captured [`FrameInputs`] against the previously
/// committed `Chrome`. Produced by
/// [`Chrome::classify`](crate::chrome::Chrome::classify); consumed by the
/// planner (`plan_frame` in `plan.rs`), which turns
/// one `FrameDelta` plus the attempt's taken `PendingWork` into a closed
/// `FramePlan` — see that module's doc comment for the complete
/// `PendingWork` x `FrameDelta` table.
#[derive(Clone)]
pub enum FrameDelta {
    Stable,
    Scroll(BlitPlan),
    Rebuild(RebuildReason),
}

/// Why [`FrameDelta::Rebuild`] fired. Named per hard-break check (rather
/// than one generic "geometry changed") so a rebuilt frame's diagnostics can
/// say which committed field diverged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebuildReason {
    NoCommittedFrame,
    Size,
    Dpr,
    Theme,
    Model,
    Sheet,
    Freeze,
    Headers,
    TwoAxisScroll,
    MissingActiveSnapshot,
    ActiveCellChangedOrUnknown,
    IncompatibleScrollOverlap,
}

/// The single pixel shift performed by a scroll blit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shift {
    pub src: PixelRect,
    pub dst: PixelRect,
}

/// 1D pixel range along a single axis (origin + size). Used by
/// `BlitPlan::for_axis_scroll` to thread main-axis and cross-axis extents
/// through axis-agnostic code without committing to X-vs-Y until the rect
/// is assembled.
#[derive(Clone, Copy)]
pub(crate) struct AxisRange {
    pub(crate) origin: i32,
    pub(crate) size: i32,
}

/// Pure-canvas-pixel description of a scroll-blit. `shift` is the one merged
/// cell-area rectangle the painter copies; `pixel_strip` is the band the
/// renderer must paint over to fill in newly-revealed
/// content. Axis tells the orchestrator which header strip to repaint
/// (the cross-axis header is untouched by the scroll).
///
/// All rects are in CSS pixels relative to the canvas origin — the
/// `Painter::blit` backend handles DPR.
#[derive(Clone)]
#[must_use = "a BlitPlan represents a committed viewport-shift decision; dropping it means the blit never happens"]
pub struct BlitPlan {
    pub axis: Axis,
    pub shift: Shift,
    pub pixel_strip: PixelRect,
}
