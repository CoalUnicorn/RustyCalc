//! Attempts view: one row per retained paint attempt.
//!
//! Rows are built by the panel; this view renders them and reports selection.
//! Holds and retries are first-class rows — a held attempt gets its own key,
//! never the number of the attempt it retried.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::perf::AttemptKey;

/// One rendered attempt row.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct AttemptRow {
    pub key: AttemptKey,
    /// The engine's attempt sequence.
    pub seq: u64,
    /// The host scope claimed for this attempt, as A1 text when addressable.
    pub scope: String,
    pub strategy: String,
    pub verdict: String,
    pub outcome: String,
    pub render_ms: Option<f64>,
    /// The attempt this one retried, by sequence.
    pub retry_of: Option<u64>,
    /// Host batches claimed for this attempt.
    pub batches: usize,
}

/// The attempt table with its Follow-latest control.
#[component]
pub(super) fn InspectorAttempts(
    rows: Memo<Vec<AttemptRow>>,
    selected: Memo<Option<AttemptKey>>,
    follow_latest: ReadSignal<bool>,
    on_select: Callback<AttemptKey>,
    on_follow_latest: Callback<bool>,
) -> impl IntoView {
    let on_follow_change = move |ev: web_sys::Event| {
        let checked = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
            .map(|input| input.checked())
            .unwrap_or(false);
        on_follow_latest.run(checked);
    };

    view! {
        <div class="pp-attempts">
            <div class="pp-attempts-head">
                <label class="pp-check">
                    <input
                        type="checkbox"
                        prop:checked=move || follow_latest.get()
                        on:change=on_follow_change
                    />
                    "Follow latest"
                </label>
                <span class="pp-attempts-count">
                    {move || format!("{} attempt(s)", rows.get().len())}
                </span>
            </div>
            <div class="pp-table-wrap">
                <table class="pp-table">
                    <thead>
                        <tr>
                            <th>"Attempt"</th>
                            <th>"Trigger scope"</th>
                            <th>"Selected strategy"</th>
                            <th>"Verdict"</th>
                            <th>"Outcome"</th>
                            <th>"Render"</th>
                            <th>"Retry of"</th>
                            <th>"Batches"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For
                            each=move || rows.get()
                            key=|row| row.key
                            children=move |row| {
                                    let key = row.key;
                                    let is_selected = move || selected.get() == Some(key);
                                    let render = row
                                        .render_ms
                                        .map_or_else(|| "\u{2014}".to_owned(), |ms| {
                                            format!("{ms:.1} ms")
                                        });
                                    let retry = row
                                        .retry_of
                                        .map_or_else(|| "\u{2014}".to_owned(), |seq| {
                                            format!("#{seq}")
                                        });
                                    let scope_text = row.scope;
                                    let scope_title = scope_text.clone();
                                    view! {
                                        <tr
                                            class:selected=is_selected
                                            on:click=move |_| on_select.run(key)
                                        >
                                            <td><button
                                                type="button"
                                                class="pp-action-btn"
                                                aria-pressed=is_selected
                                                on:click=move |_| on_select.run(key)
                                            >{format!("#{}", row.seq)}</button></td>
                                            <td title=scope_title>{scope_text}</td>
                                            <td>{row.strategy}</td>
                                            <td>{row.verdict}</td>
                                            <td>{row.outcome}</td>
                                            <td>{render}</td>
                                            <td>{retry}</td>
                                            <td>{row.batches}</td>
                                        </tr>
                                    }
                            }
                        />
                    </tbody>
                </table>
            </div>
        </div>
    }
}
