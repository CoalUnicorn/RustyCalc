#[cfg(feature = "dev-tools")]
use iron_canvas_core::PaintResult;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::DrawOp;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::RecordingFilter;
#[cfg(feature = "dev-tools")]
use iron_canvas_recorder::recording::{
    Frame, IcrHeader, RecordOrigin, RecordedPaintResult, Recording, ThemeSnapshot, TraceRecord,
};
use wasm_bindgen::prelude::*;

#[cfg(feature = "dev-tools")]
use crate::playback::PlaybackSession;

use super::IronCanvas;

#[cfg(feature = "dev-tools")]
const SOFT_WARN_MS: u64 = 30_000;
#[cfg(feature = "dev-tools")]
const HARD_CAP_BYTES: usize = 100 * 1024 * 1024;
#[cfg(feature = "dev-tools")]
const OP_BYTES_HEURISTIC: usize = 150;
#[cfg(feature = "dev-tools")]
const ATTEMPT_METADATA_BYTES_HEURISTIC: usize = 256;

// Estimate the encoded size of each operation.
// Fixed-size variants use `OP_BYTES_HEURISTIC`.
// `FillText` and `BeginGroup` also contain strings.
// Add the string lengths for these variants.
#[cfg(feature = "dev-tools")]
fn op_bytes(op: &DrawOp) -> usize {
    match op {
        DrawOp::FillText {
            text,
            font_css,
            color,
            ..
        } => OP_BYTES_HEURISTIC + text.len() + font_css.len() + color.len(),
        DrawOp::BeginGroup { class } => OP_BYTES_HEURISTIC + class.as_str().len(),
        _ => OP_BYTES_HEURISTIC,
    }
}

#[cfg(feature = "dev-tools")]
pub(super) struct RecordingState {
    rec: Recording,
    started_at_ms: f64,
    bytes_estimate: usize,
    soft_warn_fired: bool,
    capped: bool,
}

/// The lifecycle state for capture and playback.
///
/// `Live` permits normal paint operations.
/// `Recording` captures paint operations.
/// `Playback` sends recorded operations to the live painters.
#[cfg(feature = "dev-tools")]
pub(super) enum CanvasMode {
    Live,
    Recording(RecordingState),
    Playback(PlaybackSession),
}

#[wasm_bindgen]
impl IronCanvas {
    /// Return `true` if this build supports recording.
    /// JavaScript can call this method in all builds.
    #[wasm_bindgen(js_name = "recordingSupported")]
    pub fn recording_supported() -> bool {
        cfg!(feature = "dev-tools")
    }

    /// Enable or disable structured frame diagnostics.
    /// Disable the option to clear the retained snapshot.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsEnabled")]
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_enabled(enabled);
    }

    /// Set the diagnostic probe range for the next non-idle paint attempt.
    /// The snapshot identifies each planned segment that contains the range.
    /// The planner does not read this diagnostic value.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsProbe")]
    pub fn set_frame_diagnostics_probe(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_probe(iron_canvas_core::RCRange { r1, c1, r2, c2 });
    }

    /// Return structured diagnostics for the last completed live attempt.
    /// Return `undefined` if diagnostics are disabled or playback is active.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "frameDiagnostics")]
    pub fn frame_diagnostics(&self) -> JsValue {
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return JsValue::UNDEFINED;
        }
        match self.runtime.orchestrator().frame_diagnostics() {
            None => JsValue::UNDEFINED,
            Some(diag) => {
                let mut wire = crate::wire::FrameDiagnosticsWire::from(&diag);
                // Core calculates the backing size from the CSS size and DPR.
                // Use the actual grid backing size for mismatch diagnostics.
                if let Some(geometry) = &mut wire.geometry {
                    let canvas = self.runtime.grid_canvas();
                    geometry.backing_size = crate::wire::BackingSizeWire {
                        w: canvas.width(),
                        h: canvas.height(),
                    };
                }
                serde_wasm_bindgen::to_value(&wire).unwrap_or(JsValue::UNDEFINED)
            }
        }
    }

    /// Return diagnostics for the displayed recorded attempt.
    /// Return `undefined` if playback is not active.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "recordingCurrentAttempt")]
    pub fn recording_current_attempt(&self) -> Result<JsValue, JsError> {
        let CanvasMode::Playback(session) = &self.mode else {
            return Ok(JsValue::UNDEFINED);
        };
        let Some(frame) = session.recording.frames.get(session.frame_idx as usize) else {
            return Ok(JsValue::UNDEFINED);
        };
        serde_wasm_bindgen::to_value(frame)
            .map_err(|e| JsError::new(&format!("recording attempt serialization failed: {e}")))
    }

    /// Start a recording of paint operations.
    ///
    /// Return an error if recording or playback is active.
    /// `opts` is an optional JavaScript `RecordingFilter` object:
    /// `{ layers?: "both"|"gridOnly"|"overlayOnly", skipGroups?: string[] }`.
    /// `undefined` and `null` select the default filter.
    /// The layer value selects the surfaces to record.
    /// `skipGroups` excludes the specified groups and their contents.
    /// Recording stops when the host calls `stopRecording` or the size limit applies.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "startRecording")]
    pub fn start_recording(&mut self, opts: JsValue) -> Result<(), JsError> {
        match &self.mode {
            CanvasMode::Live => {}
            CanvasMode::Recording(_) => {
                return Err(JsError::new("recording already active"));
            }
            CanvasMode::Playback(_) => {
                return Err(JsError::new("cannot start a recording during playback"));
            }
        }
        // Use the default filter for `undefined` or `null`.
        // Parse all other values as `RecordingFilter`.
        let filter: RecordingFilter = if opts.is_undefined() || opts.is_null() {
            RecordingFilter::default()
        } else {
            serde_wasm_bindgen::from_value(opts)?
        };
        let canvas = self.runtime.orchestrator().canvas_size();
        let theme_snap = ThemeSnapshot::from(self.runtime.orchestrator().theme());
        let now = js_sys::Date::now();
        let header = IcrHeader::new(
            canvas.w,
            canvas.h,
            self.runtime.dpr(),
            theme_snap,
            now as u64,
        );
        self.mode = CanvasMode::Recording(RecordingState {
            rec: Recording::new(header),
            started_at_ms: now,
            bytes_estimate: 0,
            soft_warn_fired: false,
            capped: false,
        });
        if filter.layers.includes_grid() {
            self.runtime
                .orchestrator()
                .grid_surface()
                .set_skip_groups(filter.skip_groups.clone());
            self.runtime
                .orchestrator()
                .grid_surface()
                .enable_recording();
        }
        if filter.layers.includes_overlay() {
            self.runtime
                .orchestrator()
                .overlay_surface()
                .set_skip_groups(filter.skip_groups);
            self.runtime
                .orchestrator()
                .overlay_surface()
                .enable_recording();
        }
        // Request and paint the recording baseline immediately.
        // The next host rAF tick can select a narrow `SlotsReuse` paint.
        // Such a paint is not a valid replay baseline.
        // A held baseline stays in the diagnostic timeline.
        // A subsequent committed `Fresh` frame becomes the replay anchor.
        self.runtime.orchestrator_mut().request_repaint();
        self.runtime.orchestrator().grid_surface().begin_frame();
        self.runtime.orchestrator().overlay_surface().begin_frame();
        let result = self.runtime.orchestrator_mut().render_pending();
        self.capture_frame(result, RecordOrigin::ForcedBaseline);
        Ok(())
    }

    /// Stop recording and return the serialized `.icr` data.
    /// If the size limit applied, `header.partial` is `true`.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "stopRecording")]
    pub fn stop_recording(&mut self) -> Result<js_sys::Uint8Array, JsError> {
        if !matches!(self.mode, CanvasMode::Recording(_)) {
            return Err(JsError::new("no active recording"));
        }
        self.runtime
            .orchestrator()
            .grid_surface()
            .disable_recording();
        self.runtime
            .orchestrator()
            .overlay_surface()
            .disable_recording();
        let CanvasMode::Recording(state) = std::mem::replace(&mut self.mode, CanvasMode::Live)
        else {
            unreachable!("guarded above by matches!(Recording(_))")
        };
        let bytes = state
            .rec
            .serialize()
            .map_err(|e| JsError::new(&format!("recording serialize failed: {e}")))?;
        let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
        arr.copy_from(&bytes);
        Ok(arr)
    }
}

impl IronCanvas {
    /// Store one non-idle paint attempt as a frame.
    ///
    /// The frame can contain no operations when the paint attempt is held.
    /// The method updates the estimated recording size.
    /// The hard size limit sets `partial` and disables both recording surfaces.
    /// Recording state remains available so `stopRecording` can return the data.
    #[cfg(feature = "dev-tools")]
    pub(super) fn capture_frame(&mut self, result: PaintResult, origin: RecordOrigin) {
        let grid_ops = self.runtime.orchestrator().grid_surface().end_frame();
        let overlay_ops = self.runtime.orchestrator().overlay_surface().end_frame();
        if result == PaintResult::Idle {
            return;
        }
        let CanvasMode::Recording(state) = &mut self.mode else {
            return;
        };
        let now = js_sys::Date::now();
        let t_ms = (now - state.started_at_ms).max(0.0) as u64;
        let frame_idx = state.rec.frames.len() as u32;
        let frame_bytes: usize = grid_ops.iter().map(op_bytes).sum::<usize>()
            + overlay_ops.iter().map(op_bytes).sum::<usize>()
            + ATTEMPT_METADATA_BYTES_HEURISTIC;
        state.rec.push_frame(Frame {
            frame_idx,
            t_ms,
            origin,
            result: match result {
                PaintResult::Rendered => RecordedPaintResult::Painted,
                PaintResult::RetryRequired => RecordedPaintResult::Retry,
                PaintResult::Idle => unreachable!("idle attempts are omitted above"),
            },
            trace: TraceRecord::from(self.runtime.orchestrator().last_trace()),
            grid_ops,
            overlay_ops,
        });
        state.bytes_estimate = state.bytes_estimate.saturating_add(frame_bytes);

        if !state.soft_warn_fired && t_ms > SOFT_WARN_MS {
            state.soft_warn_fired = true;
            crate::wasm::diag::console_warn(
                "iron-canvas: recording is longer than 30 seconds. Call stopRecording() soon.",
            );
        }

        if !state.capped && state.bytes_estimate > HARD_CAP_BYTES {
            state.capped = true;
            state.rec.header.partial = true;
            self.runtime
                .orchestrator()
                .grid_surface()
                .disable_recording();
            self.runtime
                .orchestrator()
                .overlay_surface()
                .disable_recording();
            crate::wasm::diag::console_warn(
                "iron-canvas: recording exceeded the 100 MB limit. Recording stopped with partial data.",
            );
        }
    }

    /// Set the current theme in the active recording header.
    ///
    /// This keeps the header theme equal to the colors in subsequent operations.
    /// The method does no work when recording is not active.
    #[cfg(feature = "dev-tools")]
    pub(super) fn restamp_recording_theme(&mut self) {
        if let CanvasMode::Recording(state) = &mut self.mode {
            state.rec.header.theme = ThemeSnapshot::from(self.runtime.orchestrator().theme());
        }
    }

    #[cfg(not(feature = "dev-tools"))]
    #[inline]
    pub(super) fn restamp_recording_theme(&mut self) {
        let _ = self;
    }
}
