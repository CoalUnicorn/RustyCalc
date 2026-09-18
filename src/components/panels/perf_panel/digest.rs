//! Digest view: capture-scoped statistics under an explicit filter.
//!
//! The view renders a value resolved by the panel. It reads no store, no
//! signal, and no canvas: the filter controls report changes through
//! callbacks, and the panel recomputes the digest.
//!
//! Overlapping categories are shown as separate numbers and never added into
//! one total. A missing duration is not a zero: the render statistics always
//! show how many samples they measured.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::strategy_label;
use crate::perf::{
    CaptureDigest, EvaluationOutcome, MutationOutcome, MutationSample, RenderSample,
};
use iron_canvas_core::RenderStrategy;

/// Every strategy the effective-strategy filter can name, in a stable order.
const STRATEGIES: [RenderStrategy; 5] = [
    RenderStrategy::OverlayOnly,
    RenderStrategy::ScrollBlit,
    RenderStrategy::ChangedCells,
    RenderStrategy::FullRebuild,
    RenderStrategy::DamagedRows,
];

/// Everything the digest view renders, resolved by the panel.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DigestPanel {
    pub capture_name: Option<String>,
    pub digest: Option<CaptureDigest>,
    pub include_forced: bool,
    pub effective: Option<RenderStrategy>,
    /// Live [`PerfTimings`](crate::perf::PerfTimings) samples — the newest
    /// mutation and render call. These are not capture statistics.
    pub live_mutation: Option<MutationSample>,
    pub live_render: Option<RenderSample>,
}

/// Capture digest with its filter controls.
#[component]
pub(super) fn InspectorDigest(
    panel: Memo<DigestPanel>,
    on_include_forced: Callback<bool>,
    on_effective: Callback<Option<RenderStrategy>>,
) -> impl IntoView {
    let on_forced_change = move |ev: web_sys::Event| {
        let checked = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|input| input.checked())
            .unwrap_or(false);
        on_include_forced.run(checked);
    };

    let on_effective_change = move |ev: web_sys::Event| {
        let Some(target) = ev.target() else {
            return;
        };
        let Ok(select) = target.dyn_into::<web_sys::HtmlSelectElement>() else {
            return;
        };
        let index = select.value().parse::<usize>().unwrap_or(0);
        // Index 0 is "any"; the rest offset into `STRATEGIES`.
        on_effective.run(
            index
                .checked_sub(1)
                .and_then(|i| STRATEGIES.get(i).copied()),
        );
    };

    view! {
        <div class="pp-digest">
            <div class="pp-scope">
                <span class="pp-scope-name">
                    {move || panel.get().capture_name.unwrap_or_else(|| "no capture".to_owned())}
                </span>
                <span class="pp-scope-filter">{move || filter_scope(&panel.get())}</span>
            </div>

            <div class="pp-filters">
                <label class="pp-check">
                    <input
                        type="checkbox"
                        prop:checked=move || panel.get().include_forced
                        on:change=on_forced_change
                    />
                    "Include forced baselines"
                </label>
                <label class="pp-select">
                    "Effective strategy"
                    <select
                        prop:value=move || strategy_index(panel.get().effective).to_string()
                        on:change=on_effective_change
                    >
                        <option value="0">"any"</option>
                        {STRATEGIES
                            .iter()
                            .enumerate()
                            .map(|(index, strategy)| {
                                view! {
                                    <option value=(index + 1).to_string()>
                                        {strategy_label(*strategy)}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </label>
            </div>

            <div class="pp-live">
                {move || {
                    let panel = panel.get();
                    let mutation = panel
                        .live_mutation
                        .map_or_else(|| "\u{2014}".to_owned(), |sample| {
                            let outcome = match sample.outcome {
                                MutationOutcome::Ok => "ok",
                                MutationOutcome::Err => "err",
                            };
                            format!("{:.1} ms {outcome}", sample.apply_ms)
                        });
                    let evaluation = panel
                        .live_mutation
                        .map_or_else(|| "\u{2014}".to_owned(), |sample| match sample.evaluation {
                            EvaluationOutcome::Measured { ms } => format!("{ms:.1} ms"),
                            EvaluationOutcome::Deferred => "deferred".to_owned(),
                            EvaluationOutcome::NotRun => "not run".to_owned(),
                        });
                    let render = panel
                        .live_render
                        .map_or_else(|| "\u{2014}".to_owned(), |sample| format!("{:.1} ms", sample.ms));
                    view! {
                        <span class="pp-live-title">"Live samples"</span>
                        <span class="pp-live-cell">"mutation " {mutation}</span>
                        <span class="pp-live-cell">"eval " {evaluation}</span>
                        <span class="pp-live-cell">"render call " {render}</span>
                        <span class="pp-live-note">"not capture statistics"</span>
                    }
                }}
            </div>

            {move || match panel.get().digest {
                None => view! {
                    <p class="pp-empty">"No capture selected."</p>
                }
                .into_any(),
                Some(digest) => {
                    let render_total = format!(
                        "{:.1} ms",
                        digest.render.total_ms
                    );
                    let render_median = digest
                        .render
                        .median_ms
                        .map_or_else(|| "\u{2014}".to_owned(), |ms| format!("{ms:.1} ms"));
                    let render_max = digest
                        .render
                        .max_ms
                        .map_or_else(|| "\u{2014}".to_owned(), |ms| format!("{ms:.1} ms"));
                    view! {
                        <div class="pp-stats">
                            {stat("Attempts", digest.attempts.to_string())}
                            {stat("Committed", digest.committed.to_string())}
                            {stat("Held", digest.held.to_string())}
                            {stat("Grid paints", digest.grid_paints.to_string())}
                            {stat("Overlay-only paints", digest.overlay_only_paints.to_string())}
                            {stat("Skips", digest.skips.to_string())}
                            {stat("Fallbacks", digest.fallbacks.to_string())}
                            {stat(
                                "Render calls",
                                format!(
                                    "total {render_total} \u{b7} median {render_median} \u{b7} max {render_max}",
                                ),
                            )}
                            {stat(
                                "Render samples measured",
                                digest.render.measured.to_string(),
                            )}
                            {stat(
                                "Fetched",
                                format!(
                                    "{} addressed cells \u{b7} {} logical slots \u{b7} {} batches",
                                    digest.fetch.addressed_cells,
                                    digest.fetch.logical_slots,
                                    digest.fetch.batches,
                                ),
                            )}
                            {stat(
                                "Painted",
                                format!("{} addressed-cell visits", digest.painted_cell_visits),
                            )}
                            {stat(
                                "Mutations",
                                format!(
                                    "{} samples \u{b7} {} ok \u{b7} {} err",
                                    digest.mutation_samples, digest.mutation_ok, digest.mutation_err,
                                ),
                            )}
                            {stat(
                                "Evaluation",
                                format!(
                                    "{} measured \u{b7} {} deferred \u{b7} {} not run",
                                    digest.evaluation_measured,
                                    digest.evaluation_deferred,
                                    digest.evaluation_not_run,
                                ),
                            )}
                            {stat("Wall", format!("{:.1} ms", digest.wall_ms))}
                            {stat("Active", format!("{:.1} ms", digest.active_ms))}
                            {stat("Time in render calls", format!("{:.1} ms", digest.render_time_ms))}
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}

fn stat(label: &'static str, value: String) -> impl IntoView {
    view! {
        <div class="pp-stat">
            <span class="pp-stat-label">{label}</span>
            <span class="pp-stat-value">{value}</span>
        </div>
    }
}

/// The active filter, in one line, so the digest scope is always visible.
fn filter_scope(panel: &DigestPanel) -> String {
    let mut parts = Vec::new();
    parts.push(match panel.effective {
        Some(strategy) => format!("effective {}", strategy_label(strategy)),
        None => "all effective strategies".to_owned(),
    });
    if panel.include_forced {
        parts.push("forced baselines included".to_owned());
    }
    let mut text = parts.join(" \u{b7} ");
    if !panel.include_forced {
        let excluded = panel
            .digest
            .as_ref()
            .map_or(0, |digest| digest.forced_excluded);
        text.push_str(&format!(" \u{b7} {excluded} forced baseline(s) excluded"));
    }
    if let Some(digest) = panel.digest.as_ref() {
        if digest.truncated {
            text.push_str(" \u{b7} truncated");
        }
        if let Some(reason) = digest.stop_reason {
            text.push_str(&format!(" \u{b7} {}", reason.label()));
        }
    }
    text
}

fn strategy_index(strategy: Option<RenderStrategy>) -> usize {
    strategy
        .and_then(|strategy| {
            STRATEGIES
                .iter()
                .position(|candidate| *candidate == strategy)
        })
        .map_or(0, |index| index + 1)
}
