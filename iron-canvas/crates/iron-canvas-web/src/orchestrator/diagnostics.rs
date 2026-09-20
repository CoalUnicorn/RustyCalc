//! Canvas diagnostics accessors and projections.
//!
//! Detailed diagnostics run without paint-operation recording. The snapshot
//! type, the host capture accessors, and the JSON projection live here so
//! `recording.rs` owns the recording lifecycle only.

#[cfg(feature = "dev-tools")]
use iron_canvas_core::FrameDiagnostics;

use super::IronCanvas;
#[cfg(feature = "dev-tools")]
use super::recording::CanvasMode;

/// Owned snapshot of one completed live attempt, plus the backing-store size
/// the facade read at capture time. Not part of the JS API: the host capture
/// path holds it so export and copy can run without a live canvas.
#[cfg(feature = "dev-tools")]
#[derive(Clone, Debug)]
pub struct CanvasFrameSnapshot {
    pub diagnostics: FrameDiagnostics,
    pub backing_size: (u32, u32),
}

/// Project an immutable snapshot to a JSON object, for envelope embedding and
/// for the copy path. Touches no canvas and no orchestrator. Return `None`
/// when the projection cannot be serialized.
#[cfg(feature = "dev-tools")]
pub fn frame_diagnostics_value(snapshot: &CanvasFrameSnapshot) -> Option<serde_json::Value> {
    let wire =
        crate::wire::project_frame_diagnostics(&snapshot.diagnostics, Some(snapshot.backing_size));
    serde_json::to_value(&wire).ok()
}

#[cfg(feature = "dev-tools")]
impl IronCanvas {
    /// Return the last completed attempt with the backing size read from the
    /// live canvas. Return `None` during playback, while capture is off, or
    /// before the first enabled attempt completes.
    pub fn frame_diagnostics_snapshot(&self) -> Option<CanvasFrameSnapshot> {
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return None;
        }
        let diagnostics = self.runtime.orchestrator().frame_diagnostics()?;
        let canvas = self.runtime.grid_canvas();
        Some(CanvasFrameSnapshot {
            diagnostics,
            backing_size: (canvas.width(), canvas.height()),
        })
    }
}

impl IronCanvas {
    /// Attempt identity of the last non-idle paint, from the lightweight
    /// `FrameTrace` the renderer always publishes.
    ///
    /// Works with detailed capture disabled and allocates nothing, so a
    /// production build can number its frame trace from the engine's own
    /// attempt sequence instead of a host frame counter. Return `None` during
    /// playback and before the first non-idle attempt: `FrameTrace::default()`
    /// carries sequence zero, and the orchestrator only increments the counter
    /// for an attempt it took, so the first real attempt is 1.
    pub fn frame_attempt_seq(&self) -> Option<u64> {
        #[cfg(feature = "dev-tools")]
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return None;
        }
        let seq = self.runtime.orchestrator().last_trace().attempt_seq;
        (seq != 0).then_some(seq)
    }
}
