use leptos::prelude::*;

use crate::app_state::{AppState, ExportCmd, RecordingCmd};

/// Displays the last commit->render timing breakdown.
///
/// Shows four phases:
/// - **Input**: `set_user_input()` - writing the value into the model
/// - **Eval**: `evaluate()` - recalculating all formulas
/// - **Render**: canvas `render()` - drawing the visible grid
/// - **Total**: commit start to render complete
#[component]
pub fn PerfPanel() -> impl IntoView {
    let app = expect_context::<AppState>();
    let perf = app.perf;

    let timing = move || {
        // In / Eval are durations within the cell-commit pipeline.
        // Draw is the most recent `paintIfDirty()` duration — independent of
        // commit, so it stays meaningful even when the last action was a
        // scroll, overlay change, or theme flip.
        let commit_start = perf.commit_start.get()?;
        let input_done = perf.input_done.get()?;
        let eval_done = perf.eval_done.get()?;
        let render_ms = perf.render_ms.get()?;

        let input_ms = input_done - commit_start;
        let eval_ms = eval_done - input_done;
        let total_ms = input_ms + eval_ms + render_ms;

        Some((input_ms, eval_ms, render_ms, total_ms))
    };

    let formula_text = move || perf.last_formula.get().unwrap_or_default();

    // Runtime detect: only render the record button when the wasm was built
    // with `--features recorder`. In prod-flavor builds `recordingSupported()`
    // returns false and the button row never reaches the DOM.
    let recording_supported = iron_canvas_web::IronCanvas::recordingSupported();

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

    view! {
        <div class="pp">
            <span class="pp-label">"⏱ Perf"</span>
            {move || match timing() {
                Some((input, eval, render, total)) => {
                    view! {
                        <span class="pp-detail" title="set_user_input()">
                            {format!("In: {input:.1}ms")}
                        </span>
                        <span class="pp-detail" title="evaluate() - formula recalc">
                            {format!("Eval: {eval:.1}ms")}
                        </span>
                        <span class="pp-detail" title="Canvas render()">
                            {format!("Draw: {render:.1}ms")}
                        </span>
                        <span class="pp-total" title="Total commit-to-pixels">
                            {format!("Σ {total:.1}ms")}
                        </span>
                        <span class="pp-formula" title="Last committed formula">
                            {formula_text}
                        </span>
                    }.into_any()
                }
                None => {
                    view! {
                        <span class="pp-detail">"commit a cell to measure"</span>
                    }.into_any()
                }
            }}
            {recording_supported.then(|| view! {
                <span class="pp-sep">"|"</span>
                <button
                    class="pp-record-btn"
                    class:active=move || app.recording_active.get()
                    title="Capture paint-level .icr recording"
                    on:click=on_record_click
                >
                    {move || if app.recording_active.get() { "■ Stop" } else { "● Record" }}
                </button>
                {move || app.recording_active.get().then(|| view! {
                    <span class="pp-recording-label">"Recording…"</span>
                })}
            })}
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
// use crate::components::perf_panel::PerfPanel
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
