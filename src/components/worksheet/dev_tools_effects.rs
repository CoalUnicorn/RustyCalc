//! Dev-tools Effects: drain one-shot `AppState` command signals onto the
//! live `IronCanvas` orchestrator, mirror result state back into AppState
//! signals so the dev panels re-render.
//!
//! All three Effects share the same shape: `let-else` to drop spurious
//! re-fires from the trailing `set(None)`, `update_value` to access the
//! orchestrator, an `Err` → `StatusMessage::Error` fallthrough.

use leptos::prelude::*;

use crate::app_state::{AppState, ExportCmd, PlaybackCmd, RecordingCmd};
use crate::input::mouse::CanvasHandle;
use crate::state::{StatusMessage, WorkbookState};

/// Recording dispatch — drains `app.recording_cmd` (Start/Stop from
/// PerfPanel). `set(None)` at the end re-fires this same Effect with
/// `cmd == None`, which short-circuits via the `let-else`. No infinite
/// loop.
pub(super) fn install_recording_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
) {
    Effect::new(move |_| {
        let Some(cmd) = app.recording_cmd.get() else {
            return;
        };
        canvas_handle.update_value(|slot| {
            let Some(ic) = slot.as_mut() else {
                state
                    .status
                    .set(Some(StatusMessage::Error("canvas not ready".into())));
                return;
            };
            match cmd {
                RecordingCmd::Start => match ic.startRecording(wasm_bindgen::JsValue::UNDEFINED) {
                    Ok(()) => app.recording_active.set(true),
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "startRecording failed: {e:?}"
                    )))),
                },
                RecordingCmd::Stop => {
                    // Engine `stopRecording` clears its `recording` state before
                    // it can fail at `serialize()`; reset the UI flag eagerly so
                    // a serialize-Err doesn't wedge the button in "Stop".
                    app.recording_active.set(false);
                    match ic.stopRecording() {
                        Ok(arr) => {
                            let bytes = arr.to_vec();
                            let ts = js_sys::Date::new_0()
                                .to_iso_string()
                                .as_string()
                                .and_then(|s| s.split('.').next().map(str::to_owned))
                                .map(|s| s.replace(':', "-"))
                                .unwrap_or_else(|| "now".into());
                            let filename = format!("recording-{ts}.icr");
                            if let Err(e) = crate::input::xlsx_io::trigger_download(
                                &bytes,
                                &filename,
                                Some("application/octet-stream"),
                            ) {
                                state.status.set(Some(StatusMessage::Error(e)));
                            }
                        }
                        Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                            "stopRecording failed: {e:?}"
                        )))),
                    }
                }
            }
        });
        app.recording_cmd.set(None);
    });
}

/// Playback command dispatch — drains a one-shot `PlaybackCmd` onto the
/// live `IronCanvas`, mirrors result state back into AppState signals so
/// the PlaybackPanel re-renders.
pub(super) fn install_playback_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
    render_needed: RwSignal<bool>,
) {
    Effect::new(move |_| {
        let Some(cmd) = app.playback_cmd.get() else {
            return;
        };
        canvas_handle.update_value(|slot| {
            let Some(ic) = slot.as_mut() else {
                state
                    .status
                    .set(Some(StatusMessage::Error("canvas not ready".into())));
                return;
            };
            match cmd {
                PlaybackCmd::Load(bytes) => match ic.loadRecording(&bytes) {
                    Ok(()) => {
                        app.playback_loaded.set(true);
                        app.playback_frame_count.set(ic.recordingFrameCount());
                        app.playback_frame.set(ic.recordingCurrentFrame());
                        app.playback_playing.set(false);
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "loadRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Seek(idx) => match ic.seekRecording(idx) {
                    Ok(()) => {
                        app.playback_frame.set(ic.recordingCurrentFrame());
                        // Stage 2 invariant: seek pauses any active play loop.
                        app.playback_playing.set(false);
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "seekRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Play => match ic.playRecording(crate::perf::now()) {
                    Ok(()) => {
                        app.playback_playing.set(true);
                        // Wake the rAF: tickPlayback runs on every frame
                        // unconditionally, but a render_needed bump ensures
                        // we don't stall on a sleeping rAF after a long pause.
                        render_needed.set(true);
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "playRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Pause => {
                    ic.pauseRecording();
                    app.playback_playing.set(false);
                }
                PlaybackCmd::Exit => {
                    ic.exitPlayback();
                    app.playback_loaded.set(false);
                    app.playback_playing.set(false);
                    app.playback_frame.set(0);
                    app.playback_frame_count.set(0);
                    // exitPlayback called request_repaint on the engine; we
                    // also need a rAF wake so paintIfDirty fires next tick.
                    render_needed.set(true);
                }
            }
        });
        app.playback_cmd.set(None);
    });
}

/// Export dispatch — drains `app.export_cmd` (Svg/Pdf from PerfPanel) and
/// pipes the bytes through `trigger_download`. SVG runs today via
/// `IronCanvas::exportSvg`; the PDF arm is wired but unreachable until
/// the iron-canvas-export PDF backend lands — the PerfPanel button is
/// rendered `disabled=true` in the meantime.
pub(super) fn install_export_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
) {
    Effect::new(move |_| {
        let Some(cmd) = app.export_cmd.get() else {
            return;
        };
        canvas_handle.update_value(|slot| {
            let Some(ic) = slot.as_mut() else {
                state
                    .status
                    .set(Some(StatusMessage::Error("canvas not ready".into())));
                return;
            };
            let size = ic.canvas_size();
            let ts = js_sys::Date::new_0()
                .to_iso_string()
                .as_string()
                .and_then(|s| s.split('.').next().map(str::to_owned))
                .map(|s| s.replace(':', "-"))
                .unwrap_or_else(|| "now".into());
            match cmd {
                ExportCmd::Svg => {
                    let svg = ic.exportSvg(size.w, size.h);
                    if let Err(e) = crate::input::xlsx_io::trigger_download(
                        svg.as_bytes(),
                        &format!("sheet-{ts}.svg"),
                        Some("image/svg+xml"),
                    ) {
                        state.status.set(Some(StatusMessage::Error(e)));
                    }
                }
                ExportCmd::Pdf => {
                    let pdf = ic.exportPdf(size.w, size.h);
                    if let Err(e) = crate::input::xlsx_io::trigger_download(
                        &pdf,
                        &format!("sheet-{ts}.pdf"),
                        Some("application/pdf"),
                    ) {
                        state.status.set(Some(StatusMessage::Error(e)));
                    }
                }
            }
        });
        app.export_cmd.set(None);
    });
}
