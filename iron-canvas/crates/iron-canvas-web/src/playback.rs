//! Live-canvas playback for `.icr` recordings.
//!
//! Suspends the normal `paint_if_dirty` loop and replays recorded ops onto the
//! live grid + overlay painters. Each seek walks from the most recent `Fresh`
//! frame at or before the target — cumulative on both grid and overlay
//! surfaces. Owned by `IronCanvas`; the orchestrator is unaware.

use iron_canvas_core::PaintRegimeTag;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::{BlitPainter, Painter};
use iron_canvas_recorder::recording::{Frame, Recording};
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

pub struct PlaybackSession {
    pub recording: Recording,
    pub frame_idx: u32,
    pub clock: PlayClock,
    /// Pre-playback live canvas size, captured at `loadRecording`. The
    /// orchestrator and canvas backing stores are resized to recording
    /// dimensions for the session's lifetime; on `exitPlayback` we
    /// resize back to this.
    pub live_size: CanvasSize,
    /// Pre-playback live DPR. Mirrors `live_size`.
    pub live_dpr: f64,
}

impl PlaybackSession {
    pub fn new(recording: Recording, live_size: CanvasSize, live_dpr: f64) -> Self {
        Self {
            recording,
            frame_idx: 0,
            clock: PlayClock::Paused,
            live_size,
            live_dpr,
        }
    }

    pub fn frame_count(&self) -> u32 {
        self.recording.frames.len() as u32
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
        let frames = &self.recording.frames;
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

/// Slice index of the most recent committed `Fresh` frame at or before `target`.
///
/// `target` past the end is clamped to `frames.len() - 1` so callers can
/// pass a raw user-supplied index without pre-clamping. Returns `None` on
/// empty input, or on a malformed recording with no `Fresh` frame in range
/// — a recording may begin with held diagnostic attempts, so `None` is a
/// valid diagnostics-only prefix. A held Fresh attempt has no committed
/// sequence and therefore cannot become an anchor. Linear backward scan:
/// regime is not monotonic, so binary search does not apply, and recordings
/// tend to re-anchor frequently (resize / structural events), keeping the
/// walk short.
pub fn find_fresh_anchor(frames: &[Frame], target: u32) -> Option<u32> {
    let last = frames.len().checked_sub(1)? as u32;
    let start = target.min(last);
    (0..=start).rev().find(|&i| {
        let trace = &frames[i as usize].trace;
        trace.regime == Some(PaintRegimeTag::Fresh) && trace.committed_seq.is_some()
    })
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
pub fn replay_through<P>(
    grid: &P,
    overlay: &P,
    recording: &Recording,
    target_idx: u32,
    present_grid: &dyn Fn(),
) where
    P: Painter + BlitPainter,
{
    let frames = &recording.frames;
    if frames.is_empty() {
        return;
    }
    let target_idx = target_idx.min((frames.len() - 1) as u32);

    let Some(anchor) = find_fresh_anchor(frames, target_idx) else {
        return;
    };

    // Grid: the Fresh anchor's first ops are `ApplyDprTransform` + a
    // full-canvas fill, so no manual clear is needed before replay.
    grid.invalidate_cache();
    for frame in &frames[anchor as usize..=target_idx as usize] {
        replay(grid, &frame.grid_ops);
        present_grid();
    }

    // Overlay: cumulative from the same committed Fresh anchor. Empty ops
    // preserve the prior overlay; replaying only the target frame would lose
    // that state on backward seeks and grid-only attempts.
    overlay.invalidate_cache();
    for frame in &frames[anchor as usize..=target_idx as usize] {
        if !frame.overlay_ops.is_empty() {
            replay(overlay, &frame.overlay_ops);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iron_canvas_recorder::recording::{
        RecordOrigin, RecordedPaintResult, TraceOutcome, TraceRecord,
    };

    fn attempt(regime: Option<PaintRegimeTag>, committed_seq: Option<u64>) -> Frame {
        Frame {
            frame_idx: 0,
            t_ms: 0,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace: TraceRecord {
                attempt_seq: 1,
                committed_seq,
                regime,
                effective: regime,
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
    fn fresh_anchor_requires_a_committed_trace() {
        let frames = vec![
            attempt(Some(PaintRegimeTag::Fresh), None),
            attempt(Some(PaintRegimeTag::Fresh), Some(2)),
        ];
        assert_eq!(find_fresh_anchor(&frames, 0), None);
        assert_eq!(find_fresh_anchor(&frames, 1), Some(1));
    }

    #[test]
    fn diagnostics_only_prefix_has_no_replay_anchor() {
        let frames = vec![attempt(None, None)];
        assert_eq!(find_fresh_anchor(&frames, 0), None);
    }
}
