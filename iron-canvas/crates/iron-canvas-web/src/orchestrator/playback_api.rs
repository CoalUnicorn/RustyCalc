use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::layer::Surface;
use iron_canvas_recorder::recording::Recording;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::playback::{PlayClock, PlaybackSession, replay_through};

use super::{CanvasMode, IronCanvas};

// ============================================================================
// Recording playback (dev-tools)
//
// Playback uses the live grid painter and the live overlay painter.
// It sends the recorded `DrawOp` values to these painters.
// `paintIfDirty` bypasses the orchestrator while a session is loaded.
// `exitPlayback` requests a `Fresh` repaint for the next tick.
// ============================================================================

#[cfg(feature = "dev-tools")]
#[wasm_bindgen]
impl IronCanvas {
    /// Load `.icr` data and paint frame 0.
    /// `paintIfDirty` does no work while playback is active.
    ///
    /// The method returns an error if capture is active.
    /// Replay during capture would add replay operations to the active recording.
    ///
    /// The method sets the canvas size and DPR from the recording.
    /// It also sets the inline CSS dimensions.
    /// `exitPlayback` restores the previous size and DPR.
    #[wasm_bindgen(js_name = "loadRecording")]
    pub fn load_recording(&mut self, bytes: &[u8]) -> Result<(), JsError> {
        if matches!(self.mode, CanvasMode::Recording(_)) {
            return Err(JsError::new(
                "cannot load a recording while a capture is active",
            ));
        }
        let rec = Recording::deserialize(bytes)
            .map_err(|e| JsError::new(&format!("recording deserialize failed: {e}")))?;
        if rec.frames.is_empty() {
            return Err(JsError::new("recording has no frames"));
        }
        let live_size = self.runtime.orchestrator().canvas_size();
        let live_dpr = self.runtime.dpr();
        let rec_size = CanvasSize {
            w: rec.header.canvas_w,
            h: rec.header.canvas_h,
        };
        let rec_dpr = rec.header.dpr;

        // Set the orchestrator and backing stores to the recorded size.
        // Set the inline CSS size because `.ws-canvas` uses `100%`.
        self.runtime.resize(rec_size, rec_dpr);
        set_canvas_css_size(self.runtime.grid_canvas(), rec_size)?;
        set_canvas_css_size(self.runtime.overlay_canvas(), rec_size)?;

        self.mode = CanvasMode::Playback(PlaybackSession::new(rec, live_size, live_dpr));
        self.seek_recording_inner(0)
    }

    /// Paint the specified recorded frame.
    /// The method limits `frame_idx` to the available frame range.
    /// It pauses active playback before it paints the frame.
    ///
    /// The method returns an error if capture is active.
    #[wasm_bindgen(js_name = "seekRecording")]
    pub fn seek_recording(&mut self, frame_idx: u32) -> Result<(), JsError> {
        match &mut self.mode {
            CanvasMode::Recording(_) => {
                return Err(JsError::new(
                    "cannot seek a recording while a capture is active",
                ));
            }
            // `seek_recording_inner` returns an error because no recording is loaded.
            CanvasMode::Live => {}
            CanvasMode::Playback(s) => {
                s.clock = PlayClock::Paused;
            }
        }
        self.seek_recording_inner(frame_idx)
    }

    /// Start time-based playback.
    /// `now_ms` must be the current `performance.now()` value from the host.
    /// The method uses this value as the time anchor for subsequent ticks.
    /// The method returns an error if no recording is loaded.
    ///
    /// The method does no work at the last frame.
    /// The host can call `seekRecording(0)` to rewind the recording.
    #[wasm_bindgen(js_name = "playRecording")]
    pub fn play_recording(&mut self, now_ms: f64) -> Result<(), JsError> {
        let CanvasMode::Playback(s) = &mut self.mode else {
            return Err(JsError::new("no recording loaded"));
        };
        if s.frame_idx + 1 >= s.frame_count() {
            return Ok(());
        }
        s.anchor(now_ms);
        Ok(())
    }

    /// Pause playback. Repeated calls have no additional effect.
    #[wasm_bindgen(js_name = "pauseRecording")]
    pub fn pause_recording(&mut self) {
        if let CanvasMode::Playback(s) = &mut self.mode {
            s.clock = PlayClock::Paused;
        }
    }

    /// Return `true` while playback is active.
    /// Return `false` when playback is paused or no recording is loaded.
    #[wasm_bindgen(js_name = "isPlaying")]
    pub fn is_playing(&self) -> bool {
        matches!(
            &self.mode,
            CanvasMode::Playback(s) if matches!(s.clock, PlayClock::Playing { .. })
        )
    }

    /// Advance playback to the frame for `now_ms`.
    /// Return `true` if the displayed frame changes.
    /// Pause playback at the last frame.
    /// Do no work if playback is not active.
    ///
    /// Call this method from the rAF loop that calls `paintIfDirty`.
    /// `paintIfDirty` does not paint while playback is active.
    #[wasm_bindgen(js_name = "tickPlayback")]
    pub fn tick_playback(&mut self, now_ms: f64) -> bool {
        let CanvasMode::Playback(s) = &mut self.mode else {
            return false;
        };
        let PlayClock::Playing {
            anchor_ms,
            anchor_frame_idx,
        } = s.clock
        else {
            return false;
        };
        let target = s.target_frame_for(anchor_ms, anchor_frame_idx, now_ms);
        let last = s.frame_count().saturating_sub(1);
        let changed = target != s.frame_idx;
        if target >= last {
            s.clock = PlayClock::Paused;
        }
        if !changed {
            return false;
        }
        // The active session supplies the target and its valid frame range.
        // Therefore, `seek_recording_inner` cannot fail here.
        let _ = self.seek_recording_inner(target);
        true
    }

    /// Stop playback and request a `Fresh` repaint.
    /// Restore the canvas CSS size, canvas size, and DPR.
    /// Do no work if no playback session is loaded.
    #[wasm_bindgen(js_name = "exitPlayback")]
    pub fn exit_playback(&mut self) {
        let CanvasMode::Playback(session) = std::mem::replace(&mut self.mode, CanvasMode::Live)
        else {
            return;
        };
        // Remove the inline size so the `.ws-canvas` rule controls the size.
        let _ = clear_canvas_css_size(self.runtime.grid_canvas());
        let _ = clear_canvas_css_size(self.runtime.overlay_canvas());

        self.runtime.resize(session.live_size, session.live_dpr);
        // Playback bypasses `last_frame`. Thus, resize invalidation is not sufficient.
        self.runtime.orchestrator_mut().request_repaint();
    }

    /// Return `true` while a playback session is loaded.
    #[wasm_bindgen(js_name = "playbackActive")]
    pub fn playback_active(&self) -> bool {
        matches!(self.mode, CanvasMode::Playback(_))
    }

    /// Return the number of frames in the recording. Return 0 if none is loaded.
    #[wasm_bindgen(js_name = "recordingFrameCount")]
    pub fn recording_frame_count(&self) -> u32 {
        if let CanvasMode::Playback(s) = &self.mode {
            s.frame_count()
        } else {
            0
        }
    }

    /// Return the current frame index. Return 0 if no recording is loaded.
    #[wasm_bindgen(js_name = "recordingCurrentFrame")]
    pub fn recording_current_frame(&self) -> u32 {
        if let CanvasMode::Playback(s) = &self.mode {
            s.frame_idx
        } else {
            0
        }
    }
}

/// Set the inline canvas size in CSS pixels.
///
/// This size overrides the `100%` size from the `.ws-canvas` class.
#[cfg(feature = "dev-tools")]
fn set_canvas_css_size(canvas: &HtmlCanvasElement, size: CanvasSize) -> Result<(), JsError> {
    let style = canvas.style();
    style
        .set_property("width", &format!("{}px", size.w))
        .map_err(|_| JsError::new("failed to set canvas inline width"))?;
    style
        .set_property("height", &format!("{}px", size.h))
        .map_err(|_| JsError::new("failed to set canvas inline height"))?;
    Ok(())
}

/// Remove the inline canvas size.
/// The `.ws-canvas` CSS rule then controls the displayed size.
#[cfg(feature = "dev-tools")]
fn clear_canvas_css_size(canvas: &HtmlCanvasElement) -> Result<(), JsError> {
    let style = canvas.style();
    style
        .remove_property("width")
        .map_err(|_| JsError::new("failed to clear canvas inline width"))?;
    style
        .remove_property("height")
        .map_err(|_| JsError::new("failed to clear canvas inline height"))?;
    Ok(())
}

#[cfg(feature = "dev-tools")]
impl IronCanvas {
    fn seek_recording_inner(&mut self, frame_idx: u32) -> Result<(), JsError> {
        let CanvasMode::Playback(session) = &mut self.mode else {
            return Err(JsError::new("no recording loaded"));
        };
        let count = session.frame_count();
        if count == 0 {
            return Err(JsError::new("recording has no frames"));
        }
        let clamped = frame_idx.min(count - 1);
        session.frame_idx = clamped;

        let grid = self.runtime.orchestrator().grid_surface().painter();
        let overlay = self.runtime.orchestrator().overlay_surface().painter();
        let present_grid = || self.runtime.orchestrator().grid_surface().present();
        replay_through(grid, overlay, &session.recording, clamped, &present_grid);
        Ok(())
    }
}
