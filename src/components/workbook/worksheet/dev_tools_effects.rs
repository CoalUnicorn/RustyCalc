//! Dev-tools Effects: drain one-shot `AppState` command signals onto the
//! live `IronCanvas` orchestrator, mirror result state back into AppState
//! signals so the dev panels re-render.
//!
//! Command Effects use `let-else` to drop spurious
//! re-fires from the trailing `set(None)`, `update_value` to access the
//! orchestrator, an `Err` -> `StatusMessage::Error` fallthrough.

use leptos::prelude::*;
use std::rc::Rc;

use crate::app_state::{AppState, CaptureCmd, ExportCmd, PlaybackCmd, RecordingCmd};
use crate::events::BatchObserver;
use crate::input::mouse::CanvasHandle;
use crate::model::SheetRoster;
use crate::perf::{CaptureState, MutationSample, StopReason, attempt_json, capture_json};
use crate::state::{ModelStore, StatusMessage, WorkbookState};
#[cfg(feature = "dev-tools")]
use iron_canvas_web::ReplayResult;

/// Recording dispatch — drains `app.recording_cmd` (Start/Stop from
/// PerfPanel). `set(None)` at the end re-fires this same Effect with
/// `cmd == None`, which short-circuits via the `let-else`. No infinite
/// loop.
pub(super) fn install_recording_effect(
    state: WorkbookState,
    app: AppState,
    model: ModelStore,
    canvas_handle: CanvasHandle,
    poke: impl Fn() + Clone + 'static,
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
                RecordingCmd::Start => match ic.start_recording(wasm_bindgen::JsValue::UNDEFINED) {
                    Ok(()) => {
                        app.recording_active.set(true);
                        // Seed the overlap flag, then retain the forced
                        // baseline the recorder just painted.
                        app.perf_store.note_paint_recording();
                        super::capture_collect::append_forced_baseline(&app, model, ic);
                        // A held baseline leaves retry work queued in core.
                        // Wake the host even when its rAF loop was idle.
                        poke();
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "startRecording failed: {e:?}"
                    )))),
                },
                RecordingCmd::Stop => {
                    // Engine `stopRecording` clears its `recording` state before
                    // it can fail at `serialize()`; reset the UI flag eagerly so
                    // a serialize-Err doesn't wedge the button in "Stop".
                    app.recording_active.set(false);
                    match ic.stop_recording() {
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
    poke: impl Fn() + Clone + 'static,
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
                PlaybackCmd::Load(bytes) => match ic.load_recording(&bytes) {
                    Ok(()) => {
                        app.perf_store.finish(StopReason::PlaybackStarted);
                        app.playback_loaded.set(true);
                        app.playback_frame_count.set(ic.recording_frame_count());
                        app.playback_frame.set(ic.recording_current_frame());
                        app.playback_playing.set(false);
                        // Loading seeds frame 0 on the engine side; poke so
                        // it actually reaches the screen instead of waiting
                        // for an unrelated event to wake the (self-pausing)
                        // render loop.
                        poke();
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "loadRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Seek(idx) => match ic.seek_recording(idx) {
                    Ok(outcome) => {
                        app.playback_frame.set(ic.recording_current_frame());
                        // Stage 2 invariant: seek pauses any active play loop.
                        app.playback_playing.set(false);
                        if matches!(outcome, ReplayResult::NoCommittedFrame) {
                            // Nothing to replay before the first committed
                            // FullRebuild frame; the canvas keeps its pixels.
                            state.status.set(Some(StatusMessage::Error(
                                "no committed frame at or before this position".into(),
                            )));
                        }
                        // Seeking changes which frame the engine should show;
                        // poke so the render loop actually paints it.
                        poke();
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "seekRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Play => match ic.play_recording(crate::perf::now()) {
                    Ok(()) => {
                        app.playback_playing.set(true);
                        // Wake the (self-pausing) render loop: raf_loop.rs's
                        // playback-tick block keeps itself going every frame
                        // once playing, but the loop may currently be paused
                        // if nothing else woke it since the last idle frame.
                        poke();
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "playRecording failed: {e:?}"
                    )))),
                },
                PlaybackCmd::Pause => {
                    ic.pause_recording();
                    app.playback_playing.set(false);
                }
                PlaybackCmd::Exit => {
                    ic.exit_playback();
                    app.playback_loaded.set(false);
                    app.playback_playing.set(false);
                    app.playback_frame.set(0);
                    app.playback_frame_count.set(0);
                    // exitPlayback called request_repaint on the engine; poke
                    // so renderPending actually fires on the next frame.
                    poke();
                }
            }
        });
        app.playback_cmd.set(None);
    });
}

/// Export dispatch — drains `app.export_cmd` (Svg/Pdf from PerfPanel) and
/// pipes the bytes through `trigger_download`. Both SVG and PDF exports
/// use the current sheet view. The `dev-tools` feature enables PDF export.
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
                ExportCmd::Svg => match ic.export_svg(size.w, size.h) {
                    Ok(svg) => {
                        if let Err(e) = crate::input::xlsx_io::trigger_download(
                            svg.as_bytes(),
                            &format!("sheet-{ts}.svg"),
                            Some("image/svg+xml"),
                        ) {
                            state.status.set(Some(StatusMessage::Error(e)));
                        }
                    }
                    Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                        "exportSvg failed: {e:?}"
                    )))),
                },
                ExportCmd::Pdf => {
                    #[cfg(feature = "export")]
                    {
                        match ic.export_pdf(size.w, size.h) {
                            Ok(pdf) => {
                                if let Err(e) = crate::input::xlsx_io::trigger_download(
                                    &pdf,
                                    &format!("sheet-{ts}.pdf"),
                                    Some("application/pdf"),
                                ) {
                                    state.status.set(Some(StatusMessage::Error(e)));
                                }
                            }
                            Err(e) => state.status.set(Some(StatusMessage::Error(format!(
                                "exportPdf failed: {e:?}"
                            )))),
                        }
                    }
                    #[cfg(not(feature = "export"))]
                    {
                        state.status.set(Some(StatusMessage::Error(
                            "PDF export not enabled (build with --features export)".into(),
                        )));
                    }
                }
            }
        });
        app.export_cmd.set(None);
    });
}

/// Capture coordinator — the only code that enables or disables detailed
/// canvas capture, and the only drain of `app.capture_cmd`.
///
/// Commands only change store state. The canvas flag is derived from that
/// state in one place here, so every exit path is covered by construction:
/// pause, finish, a limit stop, a generation change, playback entry, a refused
/// start, and teardown.
///
/// The observer and the mutation sink are installed only while the capture is
/// running, so host work carries no capture cost while capture is off.
pub(super) fn install_capture_effect(
    state: WorkbookState,
    app: AppState,
    model: ModelStore,
    canvas_handle: CanvasHandle,
    poke: impl Fn() + Clone + 'static,
) {
    let store = app.perf_store;
    // The canvas flag this effect last applied. The canvas stays the authority
    // for a *successful* apply; this mirror only avoids a wasm call when the
    // desired state did not change.
    let applied = StoredValue::new_local(false);
    let attached = StoredValue::new_local(false);

    Effect::new(move |_| {
        let cmd = app.capture_cmd.get();
        let _request = store.canvas_sync_requested();
        let capture_state = store.state();

        // Playback owns the canvas: close the capture and keep it. Refusing
        // the command alone is not enough — a load can land while capturing.
        if app.playback_loaded.get()
            && matches!(
                capture_state,
                CaptureState::Capturing(_) | CaptureState::Paused(_)
            )
        {
            store.finish(StopReason::PlaybackStarted);
        }

        if let Some(cmd) = cmd {
            app.capture_cmd.set(None);
            match cmd {
                CaptureCmd::Start => {
                    let playback = app.playback_loaded.get_untracked();
                    let sheet_names = model.with_value(|model| model.get_sheet_names());
                    match store.request_start(playback, sheet_names) {
                        Ok(_) => {
                            // Seed the overlap flag: a recording that is
                            // already running covers this capture too.
                            if app.recording_active.get_untracked() {
                                store.note_paint_recording();
                            }
                        }
                        Err(refusal) => report(&state, &store, refusal.label()),
                    }
                }
                CaptureCmd::Pause => store.pause(),
                CaptureCmd::Resume => {
                    let playback = app.playback_loaded.get_untracked();
                    if let Err(refusal) = store.resume(playback) {
                        report(&state, &store, refusal.label());
                    }
                }
                CaptureCmd::Finish => store.finish(StopReason::Finished),
                CaptureCmd::ExportCaptureJson => {
                    match store.with_selected(|capture| {
                        capture_json(capture, &store.limit_report(), crate::perf::now())
                    }) {
                        Some(Ok(json)) => {
                            if let Err(message) = crate::input::xlsx_io::trigger_download(
                                json.as_bytes(),
                                &format!("perf-capture-{}.json", timestamp()),
                                Some("application/json"),
                            ) {
                                state.status.set(Some(StatusMessage::Error(message)));
                            }
                        }
                        Some(Err(error)) => state.status.set(Some(StatusMessage::Error(format!(
                            "capture export failed: {error}"
                        )))),
                        None => state
                            .status
                            .set(Some(StatusMessage::Error("no capture to export".into()))),
                    }
                }
                CaptureCmd::CopyAttemptJson(key) => {
                    match store.with_selected(|capture| {
                        capture
                            .attempts
                            .iter()
                            .find(|attempt| attempt.key == key)
                            .map(attempt_json)
                    }) {
                        Some(Some(Ok(json))) => copy_to_clipboard(&json),
                        Some(Some(Err(error))) => state.status.set(Some(StatusMessage::Error(
                            format!("attempt copy failed: {error}"),
                        ))),
                        _ => state
                            .status
                            .set(Some(StatusMessage::Error("no attempt to copy".into()))),
                    }
                }
            }
        }

        // The one place that drives the canvas flag.
        let desired = matches!(
            store.state(),
            CaptureState::Starting | CaptureState::Capturing(_)
        );
        let mut wake = false;
        if desired != applied.get_value() {
            if enable_canvas(&canvas_handle, desired) {
                applied.set_value(desired);
                if desired {
                    store.confirm_start();
                    wake = true;
                }
            } else if desired {
                store.fail_start("canvas not ready");
                report(&state, &store, "canvas not ready");
            } else {
                applied.set_value(false);
            }
        }

        // Attach the observer and the sink only while capture is running.
        let capturing = matches!(store.state(), CaptureState::Capturing(_));
        if capturing && !attached.get_value() {
            let observer: BatchObserver = Rc::new(move |batch_id, events| {
                store.note_batch(batch_id, events, |sheet| sheet_name(model, sheet));
            });
            state.events.set_batch_observer(Some(observer));
            app.perf
                .set_sample_sink(Some(Rc::new(move |sample: &MutationSample| {
                    store.note_mutation(*sample);
                })));
            attached.set_value(true);
        } else if !capturing && attached.get_value() {
            state.events.set_batch_observer(None);
            app.perf.set_sample_sink(None);
            attached.set_value(false);
        }

        if wake {
            poke();
        }
    });

    on_cleanup(move || {
        app.capture_cmd.set(None);
        state.events.set_batch_observer(None);
        app.perf.set_sample_sink(None);
        let _ = enable_canvas(&canvas_handle, false);
        app.perf_store.end_generation();
    });
}

/// Set the canvas capture flag. Returns whether a live canvas took it.
fn enable_canvas(canvas_handle: &CanvasHandle, enabled: bool) -> bool {
    let mut applied = false;
    canvas_handle.update_value(|slot| {
        if let Some(canvas) = slot.as_mut() {
            canvas.set_frame_diagnostics_enabled(enabled);
            applied = true;
        }
    });
    applied
}

/// Resolve a sheet name through the model roster, at capture time.
pub(super) fn sheet_name(model: ModelStore, sheet: u32) -> Option<String> {
    model.with_value(|model| {
        model
            .get_sheet_names()
            .into_iter()
            .find(|(id, _)| *id == sheet)
            .map(|(_, name)| name)
    })
}

/// Report a refused action without changing a published capture.
fn report(state: &WorkbookState, store: &crate::perf::PerfStore, message: &str) {
    state
        .status
        .set(Some(StatusMessage::Error(message.to_owned())));
    store.report_error(message);
}

/// Best-effort clipboard write. `write_text` returns a promise, so a denial
/// (permissions, sandboxed frame) surfaces as a rejection, not a `Result`.
fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let promise = window.navigator().clipboard().write_text(text);
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
            web_sys::console::warn_1(
                &format!("[rustycalc perf] clipboard write failed: {error:?}").into(),
            );
        }
    });
}

/// File-safe local timestamp for the export name.
fn timestamp() -> String {
    js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .and_then(|text| text.split('.').next().map(str::to_owned))
        .map(|text| text.replace(':', "-"))
        .unwrap_or_else(|| "now".into())
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::events::{ContentEvent, EventBus, SpreadsheetEvent};
    use crate::perf::{EvaluationOutcome, LimitKind, MAX_MUTATIONS, MutationOutcome};
    use iron_canvas_web::IronCanvas;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn fixture() -> (WorkbookState, AppState, ModelStore, CanvasHandle) {
        let events = EventBus::new();
        let state = WorkbookState::new(events);
        let app = AppState::new(events);
        let model = StoredValue::new_local(
            ironcalc_base::UserModel::new_empty("Sheet1", "en", "UTC", "en").unwrap(),
        );
        (state, app, model, StoredValue::new_local(None))
    }

    fn live_canvas(state: WorkbookState, model: ModelStore) -> IronCanvas {
        let make = || {
            document()
                .create_element("canvas")
                .unwrap()
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .unwrap()
        };
        let mut canvas = IronCanvas::create(make(), make()).unwrap();
        canvas.resize(400.0, 200.0, 1.0).unwrap();
        canvas.set_model(Rc::new(super::super::adapter::WorksheetModelAdapter {
            store: model,
            show_headers: state.show_headers,
        }));
        canvas
    }

    async fn flush() {
        // Drain the command, confirmation, and trailing clear notifications.
        for _ in 0..4 {
            leptos::task::tick().await;
        }
    }

    fn has_diagnostics(handle: CanvasHandle) -> bool {
        let mut captured = false;
        handle.update_value(|slot| {
            let canvas = slot.as_mut().unwrap();
            canvas.mark_content_dirty();
            canvas.render_pending();
            captured = canvas.frame_diagnostics_snapshot().is_some();
        });
        captured
    }

    fn sample(app: AppState) {
        app.perf
            .publish_mutation(0.0, 1.0, MutationOutcome::Ok, EvaluationOutcome::Deferred);
    }

    #[wasm_bindgen_test]
    async fn coordinator_confirms_dispatch_and_disables_all_capture_sources() {
        let _runtime = leptos::mount::mount_to(
            document()
                .create_element("div")
                .unwrap()
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap(),
            || (),
        );
        let owner = Owner::new();
        let (state, app, model, handle) = owner.with(fixture);
        let child = owner.with(Owner::new);
        child.with(|| install_capture_effect(state, app, model, handle, || {}));
        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert_eq!(app.perf_store.state_untracked(), CaptureState::Idle);
        assert_eq!(app.perf_store.retained_bytes(), 0);

        owner.with(|| handle.set_value(Some(live_canvas(state, model))));
        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert!(matches!(
            app.perf_store.state_untracked(),
            CaptureState::Capturing(_)
        ));
        assert!(has_diagnostics(handle));
        state
            .events
            .emit_event(SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged));
        sample(app);
        assert_eq!(app.perf_store.status_untracked().batches, 1);
        assert_eq!(app.perf_store.status_untracked().mutations, 1);

        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert!(
            matches!(app.perf_store.state_untracked(), CaptureState::Capturing(_)),
            "refusing a duplicate start must not pause capture"
        );
        app.capture_cmd.set(Some(CaptureCmd::Pause));
        flush().await;
        assert!(!has_diagnostics(handle));
        sample(app);
        state
            .events
            .emit_event(SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged));
        assert_eq!(app.perf_store.status_untracked().mutations, 1);
        assert_eq!(app.perf_store.status_untracked().batches, 1);

        app.capture_cmd.set(Some(CaptureCmd::Resume));
        flush().await;
        assert!(has_diagnostics(handle));
        for _ in 0..MAX_MUTATIONS {
            sample(app);
        }
        assert_eq!(app.perf_store.state_untracked(), CaptureState::Idle);
        flush().await;
        assert!(!has_diagnostics(handle));
        assert_eq!(
            app.perf_store.status_untracked().stop_reason,
            Some(StopReason::LimitReached(LimitKind::Mutations))
        );
        let retained = app.perf_store.retained_bytes();
        sample(app);
        assert_eq!(app.perf_store.retained_bytes(), retained);

        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert!(has_diagnostics(handle));
        app.perf_store.end_generation();
        flush().await;
        assert!(!has_diagnostics(handle));
        assert_eq!(
            app.perf_store.status_untracked().stop_reason,
            Some(StopReason::GenerationEnded)
        );

        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert!(has_diagnostics(handle));
        child.cleanup();
        assert_eq!(app.perf_store.state_untracked(), CaptureState::Idle);
        assert!(!has_diagnostics(handle));
        let retained = app.perf_store.retained_bytes();
        sample(app);
        state
            .events
            .emit_event(SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged));
        assert_eq!(app.perf_store.retained_bytes(), retained);
    }

    #[wasm_bindgen_test]
    async fn playback_finishes_capture_only_after_a_successful_load() {
        let _runtime = leptos::mount::mount_to(
            document()
                .create_element("div")
                .unwrap()
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap(),
            || (),
        );
        let owner = Owner::new();
        let (state, app, model, handle) = owner.with(fixture);
        let bytes = owner.with(|| {
            let mut canvas = live_canvas(state, model);
            canvas
                .start_recording(wasm_bindgen::JsValue::UNDEFINED)
                .unwrap();
            let bytes = canvas.stop_recording().unwrap().to_vec();
            handle.set_value(Some(canvas));
            bytes
        });
        owner.with(|| {
            install_capture_effect(state, app, model, handle, || {});
            install_playback_effect(state, app, handle, || {});
        });
        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        app.playback_cmd.set(Some(PlaybackCmd::Load(vec![1, 2, 3])));
        flush().await;
        assert!(matches!(
            app.perf_store.state_untracked(),
            CaptureState::Capturing(_)
        ));
        assert!(has_diagnostics(handle));
        app.playback_cmd.set(Some(PlaybackCmd::Load(bytes)));
        flush().await;
        assert_eq!(app.perf_store.state_untracked(), CaptureState::Idle);
        assert_eq!(
            app.perf_store.status_untracked().stop_reason,
            Some(StopReason::PlaybackStarted)
        );
        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        assert_eq!(app.perf_store.state_untracked(), CaptureState::Idle);
        app.playback_cmd.set(Some(PlaybackCmd::Exit));
        flush().await;
        assert!(!has_diagnostics(handle));
    }
    #[wasm_bindgen_test]
    async fn recording_collects_one_baseline_and_wakes_the_scheduler() {
        let _runtime = leptos::mount::mount_to(
            document()
                .create_element("div")
                .unwrap()
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap(),
            || (),
        );
        let owner = Owner::new();
        let (state, app, model, handle) = owner.with(fixture);
        let wakes = Rc::new(std::cell::Cell::new(0));
        owner.with(|| {
            handle.set_value(Some(live_canvas(state, model)));
            install_capture_effect(state, app, model, handle, || {});
            let wakes = Rc::clone(&wakes);
            install_recording_effect(state, app, model, handle, move || {
                wakes.set(wakes.get() + 1)
            });
        });
        app.capture_cmd.set(Some(CaptureCmd::Start));
        flush().await;
        app.recording_cmd.set(Some(RecordingCmd::Start));
        flush().await;
        assert_eq!(wakes.get(), 1);
        app.perf_store
            .with_active(|c| {
                assert_eq!(c.attempts.len(), 1);
                assert_eq!(
                    c.attempts[0].origin,
                    crate::perf::AttemptOrigin::ForcedBaseline
                );
                assert_eq!(c.instrumentation.forced_baselines, 1);
                assert!(c.instrumentation.paint_recording);
            })
            .unwrap();
        handle.update_value(|slot| {
            slot.as_mut().unwrap().stop_recording().unwrap();
        });
    }
}
