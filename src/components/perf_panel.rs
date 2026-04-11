use leptos::prelude::*;

use crate::app_state::AppState;

/// Displays the last commit->render timing breakdown.
///
/// Shows four phases:
/// - **Input**: `set_user_input()` - writing the value into the model
/// - **Eval**: `evaluate()` - recalculating all formulas
/// - **Render**: canvas `render()` - drawing the visible grid
/// - **Total**: commit start to render complete
#[component]
pub fn PerfPanel() -> impl IntoView {
    let perf = expect_context::<AppState>().perf;

    let timing = move || {
        let render_done = perf.render_done.get()?;
        let commit_start = perf.commit_start.get()?;
        let input_done = perf.input_done.get()?;
        let eval_done = perf.eval_done.get()?;

        let input_ms = input_done - commit_start;
        let eval_ms = eval_done - input_done;
        let render_ms = render_done - eval_done;
        let total_ms = render_done - commit_start;

        Some((input_ms, eval_ms, render_ms, total_ms))
    };

    let formula_text = move || perf.last_formula.get().unwrap_or_default();

    view! {
        <div class="perf-panel">
            <span class="perf-label">"⏱ Perf"</span>
            {move || match timing() {
                Some((input, eval, render, total)) => {
                    view! {
                        <span class="perf-detail" title="set_user_input()">
                            {format!("In: {input:.1}ms")}
                        </span>
                        <span class="perf-detail" title="evaluate() - formula recalc">
                            {format!("Eval: {eval:.1}ms")}
                        </span>
                        <span class="perf-detail" title="Canvas render()">
                            {format!("Draw: {render:.1}ms")}
                        </span>
                        <span class="perf-total" title="Total commit-to-pixels">
                            {format!("Σ {total:.1}ms")}
                        </span>
                        <span class="perf-formula" title="Last committed formula">
                            {formula_text}
                        </span>
                    }.into_any()
                }
                None => {
                    view! {
                        <span class="perf-detail">"commit a cell to measure"</span>
                    }.into_any()
                }
            }}
        </div>
    }
}
