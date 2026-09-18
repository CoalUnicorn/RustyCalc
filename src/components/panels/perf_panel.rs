use leptos::prelude::*;

#[cfg(feature = "dev-tools")]
use crate::app_state::DiagCmd;
use crate::app_state::{AppState, ExportCmd, RecordingCmd};
#[cfg(feature = "dev-tools")]
use crate::components::ui::popover::Popover;
use crate::perf::{EvaluationOutcome, MutationOutcome};
#[cfg(feature = "dev-tools")]
use wasm_bindgen::JsCast;

/// Displays the last mutation and the last paint as separate numbers.
///
/// Three readouts, each naming one thing that happened:
/// - Mutation: duration of the model closure (`mutate` / `try_mutate`) and
///   whether it applied.
/// - Eval: measured `evaluate()` duration, `deferred`, or `not run`.
/// - Render: duration of the last `render_pending()` call that painted.
///
/// Nothing here subtracts two timestamps: a phase that did not run reports
/// its own name, never a zero.
#[component]
pub fn PerfPanel() -> impl IntoView {
    let app = expect_context::<AppState>();
    let perf = app.perf;

    let mutation_text = move || {
        perf.mutation.get().map_or_else(
            || "Mutation: —".to_owned(),
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
            || "Eval: —".to_owned(),
            |sample| match sample.evaluation {
                EvaluationOutcome::Measured { ms } => format!("Eval: {ms:.1}ms"),
                EvaluationOutcome::Deferred => "Eval: deferred".to_owned(),
                EvaluationOutcome::NotRun => "Eval: not run".to_owned(),
            },
        )
    };

    let render_text = move || {
        perf.render_call.get().map_or_else(
            || "Render: —".to_owned(),
            |sample| format!("Render: {:.1}ms", sample.ms),
        )
    };

    // Which renderer path drew the last frame. Reads e.g.
    // "ChangedCells tl:skip tr:- bl:- br:FULL fetched=8000" — a `FULL` on the
    // frame right after a `ScrollBlit` is the post-blit full repaint described
    // in iron-canvas/docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md.
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

    #[cfg(feature = "dev-tools")]
    let diag_open = RwSignal::new(false);
    #[cfg(feature = "dev-tools")]
    let diag_pos = RwSignal::new((0, 0));
    #[cfg(feature = "dev-tools")]
    let diag_json = move || app.perf.frame_diagnostics.get();

    #[cfg(feature = "dev-tools")]
    let on_toggle_diag = move |ev: web_sys::MouseEvent| {
        let pos = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
            .map(|el| {
                let rect = el.get_bounding_client_rect();
                (rect.left() as i32, rect.top() as i32)
            })
            .unwrap_or((0, 0));
        diag_pos.set(pos);
        let next = !app.perf.diag_enabled.get_untracked();
        diag_open.set(next);
        app.diag_cmd.set(Some(DiagCmd::Set(next)));
    };
    #[cfg(feature = "dev-tools")]
    let on_copy_json = move |_| {
        if let Some(json) = app.perf.frame_diagnostics.get_untracked() {
            // Best-effort clipboard write. `Clipboard::write_text` returns
            // the `Promise` directly (no synchronous `Result` in web-sys),
            // so failure (denied permissions, sandboxed iframe) surfaces as
            // a promise rejection here. The text stays visible as a manual
            // fallback.
            let promise = window().navigator().clipboard().write_text(&json);
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
                    web_sys::console::warn_1(
                        &format!("[rustycalc diag] clipboard write failed: {e:?}").into(),
                    );
                }
            });
        }
    };

    // Forcing capture off on unmount: closing the Perf panel (or the
    // worksheet) must not leave detailed capture active — it would
    // contaminate later timing samples.
    #[cfg(feature = "dev-tools")]
    on_cleanup(move || app.diag_cmd.set(Some(DiagCmd::Set(false))));

    // One control governs both capture and visibility. The toggle button
    // dispatches `DiagCmd::Set(next)` itself, but the Popover's
    // outside-click close only writes `diag_open` — this effect closes that
    // gap so a dismissed popup disables capture too, instead of leaving
    // instrumentation running invisibly. Reads `diag_enabled` untracked, so
    // the effect only reacts to popup state, never to its own output.
    #[cfg(feature = "dev-tools")]
    Effect::new(move |_| {
        if !diag_open.get() && app.perf.diag_enabled.get_untracked() {
            app.diag_cmd.set(Some(DiagCmd::Set(false)));
        }
    });

    // The leptos `view!` macro does not support `#[cfg]` on child nodes (the
    // attribute is dropped, not applied), so the two diag fragments are built
    // in these closures — where `#[cfg]` is plain Rust — and spliced through
    // always-present dynamic children. In prod the body collapses to `None`
    // (renders nothing), keeping every diag reference out of that build.
    let diag_strip = move || {
        #[cfg(feature = "dev-tools")]
        {
            Some(
                view! {
                    <span class="pp-sep">"|"</span>
                    <button
                        class="pp-diag-btn"
                        class:active=move || app.perf.diag_enabled.get()
                        title="Capture structured frame diagnostics (frameDiagnostics)"
                        on:click=on_toggle_diag
                        // Stop pointerdown so the Popover's click-outside
                        // does not immediately re-close on the same event.
                        on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                    >
                        "◉ Diag"
                    </button>
                }
                .into_any(),
            )
        }
        #[cfg(not(feature = "dev-tools"))]
        {
            None::<AnyView>
        }
    };
    let diag_popover = move || {
        #[cfg(feature = "dev-tools")]
        {
            Some(
                view! {
                    <Popover
                        open=diag_open.read_only()
                        set_open=diag_open.write_only()
                        pos=diag_pos.read_only()
                        above_anchor=true
                        class="pp-diag-popover"
                    >
                        <pre class="pp-diag-json">{move || diag_json().unwrap_or_default()}</pre>
                        <button class="pp-diag-copy" on:click=on_copy_json>"Copy JSON"</button>
                    </Popover>
                }
                .into_any(),
            )
        }
        #[cfg(not(feature = "dev-tools"))]
        {
            None::<AnyView>
        }
    };

    view! {
        <div class="pp">
            <span class="pp-label">"⏱ Perf"</span>
            <span class="pp-detail" title="mutate() / try_mutate() model closure">
                {mutation_text}
            </span>
            <span class="pp-detail" title="evaluate() - formula recalc">
                {eval_text}
            </span>
            <span class="pp-detail" title="IronCanvas render_pending()">
                {render_text}
            </span>
            {move || frame_trace().map(|t| view! {
                <span class="pp-sep">"|"</span>
                <span
                    class="pp-trace"
                    title="Last frame: strategy + per-pane verdict (tl tr bl br) + cell slots fetched"
                >
                    {t}
                </span>
            })}
            {diag_popover}
            {recording_supported.then(|| view! {
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-record-btn"
                    class:active=move || app.recording_active.get()
                    disabled=move || app.playback_loaded.get()
                    title="Capture paint-level .icr recording"
                    on:click=on_record_click
                >
                    {move || if app.recording_active.get() { "■ Stop" } else { "● Record" }}
                </button>
                {move || app.recording_active.get().then(|| view! {
                    <span class="pp-recording-label">"Recording..."</span>
                })}
                {diag_strip}
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-export-btn"
                    title="Download current sheet as SVG"
                    on:click=on_export_svg
                >
                    "⇩ SVG"
                </button>
                <button
                    class="pp-export-btn"
                    title="PDF export"
                    on:click=on_export_pdf
                >
                    "⇩ PDF"
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
