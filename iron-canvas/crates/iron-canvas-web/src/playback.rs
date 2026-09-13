//! Live-canvas playback for `.icr` recordings.
//!
//! Suspends the normal `render_pending` loop and replays recorded ops onto the
//! live grid + overlay painters. Each seek walks from the most recent
//! `FullRebuild` frame at or before the target — cumulative on both grid and overlay
//! surfaces. Owned by `IronCanvas`; the orchestrator is unaware.

use iron_canvas_core::geometry::CanvasMetrics;
use iron_canvas_core::painter::{BlitPainter, Painter};
use iron_canvas_recorder::recording::{Frame, ValidatedRecording};
use iron_canvas_recorder::replay;

/// Two-state playback clock. `Paused` carries no data; `Playing` carries the
/// anchor pair (`anchor_ms` = wall-clock at last `play` / anchor reset,
/// `anchor_frame_idx` = the frame `anchor_ms` is pinned to). The target frame
/// at tick time is whichever frame's `t_ms` is closest to
/// `frames[anchor_frame_idx].t_ms + (now - anchor_ms)`.
pub enum PlayClock {
    Paused,
    Playing {
        anchor_ms: f64,
        anchor_frame_idx: u32,
    },
}

/// What a replay actually painted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplayOutcome {
    /// The anchor and every frame up to the target replayed onto the live
    /// painters.
    Replayed,
    /// No committed `FullRebuild` frame exists at or before the target, so
    /// there was nothing to replay: the canvas keeps the pixels it already
    /// had. A recording may begin with held attempts, so a diagnostics-only
    /// prefix legitimately ends here.
    NoCommittedFrame,
}

pub struct PlaybackSession {
    pub recording: ValidatedRecording,
    pub frame_idx: u32,
    pub clock: PlayClock,
    /// Pre-playback live canvas metrics, captured at `loadRecording`. The
    /// orchestrator and canvas backing stores are resized to the recording's
    /// dimensions for the session's lifetime; on `exitPlayback` we resize back
    /// to this.
    pub live_metrics: CanvasMetrics,
}

impl PlaybackSession {
    pub fn new(recording: ValidatedRecording, live_metrics: CanvasMetrics) -> Self {
        Self {
            recording,
            frame_idx: 0,
            clock: PlayClock::Paused,
            live_metrics,
        }
    }

    pub fn frame_count(&self) -> u32 {
        self.recording.frame_count()
    }

    /// Pin the playback clock and enter `Playing`: subsequent
    /// `target_frame_for` calls measure elapsed time from `now_ms` against
    /// the timestamp of the current `frame_idx`.
    pub fn anchor(&mut self, now_ms: f64) {
        self.clock = PlayClock::Playing {
            anchor_ms: now_ms,
            anchor_frame_idx: self.frame_idx,
        };
    }

    /// Slice index of the frame that should be on screen at `now_ms`.
    /// Caller passes the anchor pair (destructured from `PlayClock::Playing`)
    /// so this method is only reachable in the playing state. Walks forward
    /// only — playback is strictly monotonic. Returns the anchor index when
    /// no frame has elapsed yet.
    pub fn target_frame_for(&self, anchor_ms: f64, anchor_frame_idx: u32, now_ms: f64) -> u32 {
        let frames = self.recording.frames();
        if frames.is_empty() {
            return 0;
        }
        let count = frames.len() as u32;
        let anchor_t = frames[anchor_frame_idx as usize].t_ms as f64;
        let target_t = anchor_t + (now_ms - anchor_ms);
        let mut i = self.frame_idx.max(anchor_frame_idx);
        while i + 1 < count && (frames[(i + 1) as usize].t_ms as f64) <= target_t {
            i += 1;
        }
        i
    }
}

/// Slice index of the most recent committed `FullRebuild` frame at or before
/// `target`.
///
/// `target` past the end is clamped to `frames.len() - 1` so callers can
/// pass a raw user-supplied index without pre-clamping. Returns `None` on
/// empty input, or on a malformed recording with no `FullRebuild` frame in range
/// — a recording may begin with held diagnostic attempts, so `None` is a
/// valid diagnostics-only prefix. A held `FullRebuild` attempt has no committed
/// sequence and therefore cannot become an anchor. Linear backward scan:
/// strategy is not monotonic, so binary search does not apply, and recordings
/// tend to re-anchor frequently (resize / structural events), keeping the
/// walk short.
pub fn find_full_rebuild_anchor(frames: &[Frame], target: u32) -> Option<u32> {
    let last = frames.len().checked_sub(1)? as u32;
    let start = target.min(last);
    (0..=start)
        .rev()
        .find(|&i| frames[i as usize].is_replay_anchor())
}

/// Replay cumulative grid and overlay state for `target_idx`
/// onto the live painters.
///
/// Generic over `Painter + BlitPainter` so it works against both the bare
/// `CanvasPainter` and the dev-tools `RecordingPainter<CanvasPainter>` that
/// `RecordingSurface` returns from `painter()`.
///
/// `present_grid` is called after **every** replayed grid frame, not once
/// at the end. `CanvasPainter::blit` reads its kept band from the
/// *visible front* canvas, while replay paints into the detached back
/// canvas — a `Blit` op replayed before its predecessor's pixels are
/// presented reads stale/cleared front pixels and corrupts the composite.
/// Mirrors the live loop, which presents after every painted frame.
///
/// Returns [`ReplayOutcome::NoCommittedFrame`] — rather than silently doing
/// nothing — when the recording has no committed `FullRebuild` frame at or
/// before the target. That state is legitimate (a diagnostics-only prefix),
/// so it is a reported outcome, not an error.
pub fn replay_through<P>(
    grid: &P,
    overlay: &P,
    recording: &ValidatedRecording,
    target_idx: u32,
    present_grid: &dyn Fn(),
) -> ReplayOutcome
where
    P: Painter + BlitPainter,
{
    let frames = recording.frames();
    let target_idx = target_idx.min((frames.len() - 1) as u32);

    let Some(anchor) = find_full_rebuild_anchor(frames, target_idx) else {
        return ReplayOutcome::NoCommittedFrame;
    };

    // Grid: the FullRebuild anchor's first ops are `ApplyDprTransform` + a
    // full-canvas fill, so no manual clear is needed before replay.
    grid.invalidate_cache();
    for frame in &frames[anchor as usize..=target_idx as usize] {
        replay(grid, &frame.grid_ops);
        present_grid();
    }

    // Overlay: cumulative from the same committed FullRebuild anchor. Empty ops
    // preserve the prior overlay; replaying only the target frame would lose
    // that state on backward seeks and grid-only attempts.
    overlay.invalidate_cache();
    for frame in &frames[anchor as usize..=target_idx as usize] {
        if !frame.overlay_ops.is_empty() {
            replay(overlay, &frame.overlay_ops);
        }
    }
    ReplayOutcome::Replayed
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_core::RenderStrategy;
    use iron_canvas_recorder::recording::{
        RecordOrigin, RecordedPaintResult, TraceOutcome, TraceRecord,
    };

    fn attempt(strategy: Option<RenderStrategy>, committed_seq: Option<u64>) -> Frame {
        Frame {
            frame_idx: 0,
            t_ms: 0,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace: TraceRecord {
                attempt_seq: 1,
                committed_seq,
                strategy,
                effective: strategy,
                work: 0,
                verdict: None,
                outcome: TraceOutcome::Painted,
                blit_fallback: None,
                fetched_cell_slots: 0,
                fetched_cells: 0,
                fetch_batches: 0,
            },
            grid_ops: Vec::new(),
            overlay_ops: Vec::new(),
        }
    }

    #[test]
    fn full_rebuild_anchor_requires_a_committed_trace() {
        let frames = vec![
            attempt(Some(RenderStrategy::FullRebuild), None),
            attempt(Some(RenderStrategy::FullRebuild), Some(2)),
        ];
        assert_eq!(find_full_rebuild_anchor(&frames, 0), None);
        assert_eq!(find_full_rebuild_anchor(&frames, 1), Some(1));
    }

    #[test]
    fn diagnostics_only_prefix_has_no_replay_anchor() {
        let frames = vec![attempt(None, None)];
        assert_eq!(find_full_rebuild_anchor(&frames, 0), None);
    }

    #[test]
    fn fallback_reanchors_but_a_held_attempt_does_not() {
        let mut fallback = attempt(Some(RenderStrategy::ScrollBlit), Some(2));
        fallback.trace.effective = Some(RenderStrategy::FullRebuild);
        let mut held = attempt(Some(RenderStrategy::FullRebuild), Some(3));
        held.trace.outcome = TraceOutcome::HeldOnBridgeFailure;
        let frames = vec![fallback, held];
        assert_eq!(find_full_rebuild_anchor(&frames, 1), Some(0));
    }
}
