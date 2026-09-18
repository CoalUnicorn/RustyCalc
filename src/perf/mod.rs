//! Performance instrumentation for the dev-tools inspector.
//!
//! `timing.rs` holds the typed samples the panel reads: one completed
//! mutation per model wrapper call, and one measured render call per painted
//! frame. Timestamps come from `performance.now()` (sub-millisecond
//! resolution).
//!
//! `capture.rs` holds the immutable capture records and the pure archive that
//! retains them; `store.rs` puts signals in front of it; `export.rs` writes
//! the JSON envelope. All three are `dev-tools` only, so a production build
//! retains no capture state.
//!
//! The panel shows each sample as its own number. It never subtracts two
//! timestamps, so a phase that did not run can never be reported as a zero
//! duration.

#[cfg(feature = "dev-tools")]
mod capture;
#[cfg(feature = "dev-tools")]
mod export;
#[cfg(feature = "dev-tools")]
mod store;
mod timing;

#[cfg(feature = "dev-tools")]
pub use capture::{
    AppendOutcome, AttemptKey, AttemptOrigin, AttemptRecord, BatchFacts, CaptureArchive, CaptureId,
    CaptureRecord, CaptureState, CaptureStatus, HostBatchId, HostBatchKind, HostBatchSummary,
    HostScope, InstrumentationFlags, LimitKind, LimitReport, MAX_ATTEMPTS, MAX_BATCHES, MAX_BYTES,
    MAX_CAPTURES, MAX_MUTATIONS, SheetRef, StartRefusal, StopReason, estimate_attempt_bytes,
    estimate_capture_bytes, estimate_host_summary_bytes, summarize,
};
#[cfg(feature = "dev-tools")]
pub use export::{
    AttemptWire, CAPTURE_ENVELOPE_VERSION, CaptureEnvelopeWire, CaptureWire, ExportError,
    attempt_json, capture_json,
};
#[cfg(feature = "dev-tools")]
pub use store::PerfStore;
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
