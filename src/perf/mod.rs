//! Performance instrumentation for the dev-tools Perf panel.
//!
//! `timing.rs` holds the typed samples the panel reads: one completed
//! mutation per model wrapper call, and one measured render call per painted
//! frame. Timestamps come from `performance.now()` (sub-millisecond
//! resolution).
//!
//! The panel shows each sample as its own number. It never subtracts two
//! timestamps, so a phase that did not run can never be reported as a zero
//! duration.

mod timing;

#[cfg(feature = "dev-tools")]
pub use timing::SampleSink;
pub use timing::{EvaluationOutcome, MutationOutcome, MutationSample, PerfTimings, RenderSample};

/// Read `performance.now()` from the browser.
pub fn now() -> f64 {
    leptos::prelude::window()
        .performance()
        .map(|p| p.now())
        .unwrap_or(0.0)
}
