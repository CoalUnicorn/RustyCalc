//! The frame report: what one paint attempt did.
//!
//! `FrameOutcome`, `GridVerdict`, `FrameTrace`, and `BlitFallback` are the
//! renderer-facing result types, stamped into `Orchestrator.last_trace` at
//! the end of `render_pending`.

use std::fmt;

use crate::frame::inputs::FrameInputFailure;
use crate::frame::plan::RenderStrategy;
use crate::frame::work::WorkFlags;

/// The result of one `render_pending` call.
/// `RetryRequired` means that the scheduler must keep the loop active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintResult {
    Idle,
    Rendered,
    RetryRequired,
}

/// What the grid paint decided this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridVerdict {
    Skip,
    Cell,
    Range,
    Rows {
        spans: u8,
        rows: u16,
    },
    Full,
    Strip,
    /// Grid-wide preflight held the prior buffers and pixels.
    Held,
}

impl fmt::Display for GridVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skip => f.write_str("skip"),
            Self::Cell => f.write_str("cell"),
            Self::Range => f.write_str("range"),
            Self::Rows { spans, rows } => write!(f, "rows{spans}/{rows}"),
            Self::Full => f.write_str("FULL"),
            Self::Strip => f.write_str("strip"),
            Self::Held => f.write_str("held"),
        }
    }
}

/// Whole-frame outcome. Blit preflight validates every required address strip
/// before the caller shifts a single pixel, so any bridge failure holds the
/// grid transaction without calling `Painter::blit`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameOutcome {
    #[default]
    Painted,
    HeldOnBridgeFailure,
    /// `FrameInputs::capture` failed before dispatch reached a strategy at
    /// all — no candidate geometry, no cache invalidation, no paint. See
    /// `render_pending`'s capture-failure handling.
    HeldOnInputFailure(FrameInputFailure),
}

/// A blit whose committed cache could not be shifted, so preparation fell back
/// to a full-grid replacement. A cold cache and an incompatible layout have
/// different diagnostic causes even though both use that replacement path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlitFallback {
    pub cold_cache: bool,
}

/// Per-frame attribution: which strategy ran, what the grid decided, and how
/// much model traffic it cost. Written by the renderer during paint, stamped
/// into `Orchestrator.last_trace` at the end of `render_pending`.
///
/// Exists to answer "which path painted this frame?" without a code read —
/// specifically whether a post-blit `ChangedCells` strategy reports `Full`.
/// hypothesis in `docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameTrace {
    /// Monotonically wrapping identifier for each non-idle paint attempt.
    /// Capture holds receive an id too, so recorder diagnostics can correlate
    /// a retry with the attempt that produced it.
    pub attempt_seq: u64,
    /// Identifier of the successful transaction committed by this attempt.
    /// Holds leave this unset.
    pub committed_seq: Option<u64>,
    /// `None` before the first dispatch and on a capture hold. `RenderStrategy`
    /// has no `Default` on purpose. A default would name a strategy that
    /// never ran.
    pub strategy: Option<RenderStrategy>,
    /// The strategy that painted pixels in this frame. It is equal to
    /// `strategy` unless a `ScrollBlit` rejects in-place reuse and uses
    /// `BlitOutcome::FreshFallback`. `None` means that no paint completed.
    pub effective: Option<RenderStrategy>,
    /// Diagnostic projection of the `PendingWork` snapshot `plan_frame`
    /// acted on. The strategy alone cannot explain the decision.
    /// `ChangedCells` is the fallback arm, so it identifies rejected
    /// arms were *rejected* only once you know which categories carried
    /// work.
    pub work: WorkFlags,
    /// `None` when the grid was not visited this frame.
    pub verdict: Option<GridVerdict>,
    pub outcome: FrameOutcome,
    /// Set when a `ScrollBlit` frame had to abandon cache shifting and prepare a
    /// full-grid replacement on a frame expected to repaint only a strip.
    pub blit_fallback: Option<BlitFallback>,
    /// Cell slots handed to the model, summed over the bundle channels and
    /// counted per call. A full-grid blit fallback adopts the buffers its
    /// preflight already validated instead of refetching the same cells.
    pub fetched_cell_slots: usize,
    /// Distinct addressed cells charged by the renderer's bundle fetches.
    /// Unlike `fetched_cell_slots`, this does not encode the current number of
    /// model channels.
    pub fetched_cells: usize,
    /// Number of renderer-owned bundle fetches represented by this trace.
    /// This is not a host-call count; adapter internals may still perform
    /// scalar reads behind one bundle request.
    pub fetch_batches: usize,
}

impl fmt::Display for FrameTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.strategy {
            Some(r) => write!(f, "{r:?}")?,
            None => f.write_str("-")?,
        }
        write!(f, "[{:?}]", self.work)?;
        match self.verdict {
            Some(verdict) => write!(f, " grid:{verdict}")?,
            None => f.write_str(" grid:-")?,
        }
        if self.outcome == FrameOutcome::HeldOnBridgeFailure {
            f.write_str(" HELD")?;
        }
        if let Some(fb) = self.blit_fallback {
            let why = if fb.cold_cache { "cold" } else { "range" };
            write!(f, " unshift({why})")?;
        }
        write!(f, " fetched={}", self.fetched_cell_slots)?;
        // Only printed on divergence (a `FreshFallback`) so the ordinary
        // line stays exactly as short as before this field existed.
        if self.effective != self.strategy {
            match self.effective {
                Some(e) => write!(f, " eff:{e:?}")?,
                None => f.write_str(" eff:-")?,
            }
        }
        Ok(())
    }
}
