use leptos::prelude::*;

#[cfg(feature = "dev-tools")]
use crate::app_state::CaptureCmd;
use crate::app_state::{AppState, ExportCmd, RecordingCmd};
#[cfg(feature = "dev-tools")]
use crate::components::ui::popover::Popover;
#[cfg(feature = "dev-tools")]
use crate::perf::{CaptureState, attempt_json};
use crate::perf::{EvaluationOutcome, MutationOutcome};
#[cfg(feature = "dev-tools")]
use wasm_bindgen::JsCast;

/// Dev-tools strip: the last mutation, evaluation, and render numbers, plus
/// the capture controls.
///
/// Three readouts, each naming one thing that happened:
/// - Mutation: duration of the model closure (`mutate` / `try_mutate`) and
///   whether it applied.
/// - Eval: measured `evaluate()` duration, `deferred`, or `not run`.
/// - Render: duration of the last `render_pending()` call that painted.
///
/// Nothing here subtracts two timestamps: a phase that did not run reports its
/// own name, never a zero.
///
/// The capture controls publish intent to `CaptureCmd` only. They never touch
/// the canvas, and closing the popover does not change capture state — the
/// capture outlives any view of it.
#[component]
pub fn PerfPanel() -> impl IntoView {
    let app = expect_context::<AppState>();
    let perf = app.perf;

    let mutation_text = move || {
        perf.mutation.get().map_or_else(
            || "Mutation: \u{2014}".to_owned(),
            |sample| {
                let outcome = match sample.outcome {
                    MutationOutcome::Ok => "ok",
                    MutationOutcome::Err => "err",
                };
                format!("Mutation: {:.1}ms {outcome}", sample.apply_ms)
            },
        )
    };

    let eval_text = move || {
        perf.mutation.get().map_or_else(
            || "Eval: \u{2014}".to_owned(),
            |sample| match sample.evaluation {
                EvaluationOutcome::Measured { ms } => format!("Eval: {ms:.1}ms"),
                EvaluationOutcome::Deferred => "Eval: deferred".to_owned(),
                EvaluationOutcome::NotRun => "Eval: not run".to_owned(),
            },
        )
    };

    let render_text = move || {
        perf.render_call.get().map_or_else(
            || "Render: \u{2014}".to_owned(),
            |sample| format!("Render: {:.1}ms", sample.ms),
        )
    };

    // Which renderer path drew the last frame. Reads e.g.
    // "ChangedCells tl:skip tr:- bl:- br:FULL fetched=8000", numbered by the
    // engine's attempt sequence.
    let frame_trace = move || perf.frame_trace.get();

    // Runtime detect: only render the record button when the wasm was built
    // with `--features dev-tools`. In prod-flavor builds `recording_supported()`
    // returns false and the button row never reaches the DOM.
    let recording_supported = iron_canvas_web::IronCanvas::recording_supported();

    let on_record_click = move |_| {
        let cmd = if app.recording_active.get() {
            RecordingCmd::Stop
        } else {
            RecordingCmd::Start
        };
        app.recording_cmd.set(Some(cmd));
    };

    let on_export_svg = move |_| app.export_cmd.set(Some(ExportCmd::Svg));
    let on_export_pdf = move |_| app.export_cmd.set(Some(ExportCmd::Pdf));

    // ---- capture controls (dev-tools only) ----
    #[cfg(feature = "dev-tools")]
    let store = app.perf_store;
    #[cfg(feature = "dev-tools")]
    let popover_open = RwSignal::new(false);
    #[cfg(feature = "dev-tools")]
    let popover_pos = RwSignal::new((0, 0));

    #[cfg(feature = "dev-tools")]
    let capture_label = move || {
        let status = store.status();
        let mut text = format!(
            "{} \u{b7} {} attempts",
            status.state.label(),
            status.attempts
        );
        if let Some(reason) = status.stop_reason {
            text.push_str(&format!(" \u{b7} {}", reason.label()));
        }
        if let Some(error) = store.error() {
            text.push_str(&format!(" \u{b7} {error}"));
        }
        text
    };

    // The JSON of the newest attempt in the selected capture: the record is
    // the source of truth, not the live canvas.
    #[cfg(feature = "dev-tools")]
    let attempt_json_text = move || {
        if !popover_open.get() {
            return String::new();
        }
        let _revision = store.revision();
        store
            .with_latest_attempt(attempt_json)
            .and_then(Result::ok)
            .unwrap_or_default()
    };

    #[cfg(feature = "dev-tools")]
    let on_toggle_popover = move |ev: web_sys::MouseEvent| {
        let pos = ev
            .current_target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
            .map(|element| {
                let rect = element.get_bounding_client_rect();
                (rect.left() as i32, rect.top() as i32)
            })
            .unwrap_or((0, 0));
        popover_pos.set(pos);
        popover_open.update(|open| *open = !*open);
    };

    #[cfg(feature = "dev-tools")]
    let send = move |cmd: CaptureCmd| app.capture_cmd.set(Some(cmd));

    #[cfg(feature = "dev-tools")]
    let capture_strip = move || {
        let status = store.status();
        let capturing = matches!(status.state, CaptureState::Capturing(_));
        let paused = matches!(status.state, CaptureState::Paused(_));
        let idle = matches!(status.state, CaptureState::Idle);
        Some(
            view! {
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-cap-btn"
                    disabled=move || !idle
                    title="Start a paint-attempt capture"
                    on:click=move |_| send(CaptureCmd::Start)
                >
                    "\u{25cf} Start"
                </button>
                <button
                    class="pp-cap-btn"
                    disabled=move || !capturing
                    title="Stop accepting records; every record is kept"
                    on:click=move |_| send(CaptureCmd::Pause)
                >
                    "\u{23f8} Pause"
                </button>
                <button
                    class="pp-cap-btn"
                    disabled=move || !paused
                    title="Accept records again"
                    on:click=move |_| send(CaptureCmd::Resume)
                >
                    "\u{25b6} Resume"
                </button>
                <button
                    class="pp-cap-btn"
                    disabled=move || idle
                    title="Close the capture and keep it in the archive"
                    on:click=move |_| send(CaptureCmd::Finish)
                >
                    "\u{23f9} Finish"
                </button>
                <button
                    class="pp-cap-btn"
                    disabled=move || status.selected.is_none()
                    title="Download the selected capture as JSON"
                    on:click=move |_| send(CaptureCmd::ExportCaptureJson)
                >
                    "\u{2913} JSON"
                </button>
                <button
                    class="pp-cap-btn"
                    class:active=move || popover_open.get()
                    disabled=move || status.attempts == 0
                    title="Copy the newest attempt JSON"
                    on:click=move |_| send(CaptureCmd::CopySelectedAttemptJson)
                >
                    "Copy attempt"
                </button>
                <button
                    class="pp-diag-btn"
                    class:active=move || popover_open.get()
                    title="Show the newest attempt JSON"
                    on:click=on_toggle_popover
                    // Stop pointerdown so the Popover's click-outside does not
                    // immediately re-close on the same event.
                    on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                >
                    "\u{25c9} JSON"
                </button>
                <span class="pp-detail" title="Capture state, retained attempts, and the stop reason">
                    {capture_label}
                </span>
            }
            .into_any(),
        )
    };
    #[cfg(not(feature = "dev-tools"))]
    let capture_strip = move || None::<AnyView>;

    #[cfg(feature = "dev-tools")]
    let json_popover = move || {
        Some(
            view! {
                <Popover
                    open=popover_open.read_only()
                    set_open=popover_open.write_only()
                    pos=popover_pos.read_only()
                    above_anchor=true
                    class="pp-diag-popover"
                >
                    <pre class="pp-diag-json">{attempt_json_text}</pre>
                </Popover>
            }
            .into_any(),
        )
    };
    #[cfg(not(feature = "dev-tools"))]
    let json_popover = move || None::<AnyView>;

    view! {
        <div class="pp">
            <span class="pp-label">"\u{23f1} Perf"</span>
            <span class="pp-detail" title="mutate() / try_mutate() model closure">
                {mutation_text}
            </span>
            <span class="pp-detail" title="evaluate() - formula recalc">
                {eval_text}
            </span>
            <span class="pp-detail" title="IronCanvas render_pending()">
                {render_text}
            </span>
            {move || frame_trace().map(|trace| view! {
                <span class="pp-sep">"|"</span>
                <span
                    class="pp-trace"
                    title="Last attempt: strategy + per-pane verdict (tl tr bl br) + cell slots fetched"
                >
                    {trace}
                </span>
            })}
            {json_popover}
            {capture_strip}
            {recording_supported.then(|| view! {
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-record-btn"
                    class:active=move || app.recording_active.get()
                    disabled=move || app.playback_loaded.get()
                    title="Capture paint-level .icr recording"
                    on:click=on_record_click
                >
                    {move || if app.recording_active.get() { "\u{25a0} Stop" } else { "\u{25cf} Record" }}
                </button>
                {move || app.recording_active.get().then(|| view! {
                    <span class="pp-recording-label">"Recording..."</span>
                })}
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-export-btn"
                    title="Download current sheet as SVG"
                    on:click=on_export_svg
                >
                    "\u{21e9} SVG"
                </button>
                <button
                    class="pp-export-btn"
                    title="PDF export"
                    on:click=on_export_pdf
                >
                    "\u{21e9} PDF"
                </button>
            })}
        </div>
    }
}

// usage code:
//
// use crate::app_state::AppState;
//
// Component
// ```rust
//
// use crate::components::panels::perf_panel::PerfPanel
// pub fn Bar() -> impl IntoView {
//   let state = expect_context::<WorkbookState>();
//   let app = expect_context::<AppState>();
//   let model = expect_context::<ModelStore>();
//
//   let on_toggle_perf = move || {
//      app.show_perf_panel.update(|v| *v = !*v);
//   };
// ```

// let perf_label = move || {
//     if app.show_perf_panel.get() {
//         "Hide perf panel"
//     } else {
//         "Show perf panel"
//     }
// };
//
//   view!{
//    <Show when=move || app.show_perf_panel.get()>
//          <PerfPanel />
//      </Show>
//  }
//
