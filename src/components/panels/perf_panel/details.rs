//! Details view: one attempt's address evidence, then its raw sections.
//!
//! Address evidence comes first because it answers the question the inspector
//! exists for: what did this attempt actually touch? Cache, geometry, blit,
//! work flags, probe, and the raw frame trace sit behind disclosure controls,
//! because they are diagnostic detail rather than the summary.
//!
//! Every list names its source and its precision. An unavailable list shows
//! the reason — never an empty list that reads as "nothing happened".

use leptos::prelude::*;

/// One evidence row, pre-formatted by the panel.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct EvidenceRow {
    pub label: &'static str,
    /// `exact`, `derived`, or the recorded unavailability reason.
    pub precision: String,
    pub note: Option<String>,
    pub addresses: Vec<String>,
    /// A CSS-pixel rectangle, kept out of the address list.
    pub clip: Option<String>,
}

/// One disclosure section of raw diagnostic facts.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AdvancedSection {
    pub title: &'static str,
    pub rows: Vec<(String, String)>,
}

/// Everything the details view renders.
#[derive(Clone, Debug, PartialEq, Default)]
pub(super) struct DetailsView {
    /// The selected attempt, as `#781`.
    pub attempt: Option<String>,
    pub evidence: Vec<EvidenceRow>,
    pub advanced: Vec<AdvancedSection>,
    /// Latest live trace. It does not belong to the selected attempt.
    pub frame_trace: Option<String>,
}

/// Address evidence and advanced sections for the selected attempt.
#[component]
pub(super) fn InspectorDetails(view: Memo<DetailsView>) -> impl IntoView {
    view! {
        <div class="pp-details">
            {move || match view.get().attempt {
                None => view! { <p class="pp-empty">"No attempt selected."</p> }.into_any(),
                Some(attempt) => {
                    let view = view.get();
                    let evidence = view.evidence;
                    let advanced = view.advanced;
                    let frame_trace = view.frame_trace;
                    view! {
                        <div class="pp-detail-head">
                            <span class="pp-detail-attempt">{attempt}</span>
                        </div>
                        <div class="pp-evidence">
                            {evidence
                                .into_iter()
                                .map(|row| {
                                    view! {
                                        <div class="pp-evidence-row">
                                            <div class="pp-evidence-head">
                                                <span class="pp-evidence-label">{row.label}</span>
                                                <span class="pp-badge">{row.precision}</span>
                                            </div>
                                            {row.note
                                                .map(|note| view! {
                                                    <div class="pp-evidence-note">{note}</div>
                                                })}
                                            {(!row.addresses.is_empty())
                                                .then(|| view! {
                                                    <ul class="pp-address-list">
                                                        {row.addresses
                                                            .into_iter()
                                                            .map(|address| view! { <li>{address}</li> })
                                                            .collect_view()}
                                                    </ul>
                                                })}
                                            {row.clip
                                                .map(|clip| view! {
                                                    <div class="pp-evidence-clip">
                                                        "clip " {clip}
                                                    </div>
                                                })}
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </div>
                        <div class="pp-advanced">
                            {advanced
                                .into_iter()
                                .map(|section| {
                                    view! {
                                        <details class="pp-section">
                                            <summary>{section.title}</summary>
                                            <dl class="pp-kv">
                                                {section
                                                    .rows
                                                    .into_iter()
                                                    .map(|(key, value)| {
                                                        view! {
                                                            <dt>{key}</dt>
                                                            <dd>{value}</dd>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </dl>
                                        </details>
                                    }
                                })
                                .collect_view()}
                            {frame_trace
                                .map(|trace| view! {
                                    <details class="pp-section">
                                        <summary>"Latest live frame trace (not selected attempt)"</summary>
                                        <pre class="pp-trace-raw">{trace}</pre>
                                    </details>
                                })}
                        </div>
                    }
                    .into_any()
                }
            }}
        </div>
    }
}
