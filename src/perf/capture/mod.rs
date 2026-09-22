//! Immutable capture records, host-event projection, and the retaining archive.
//!
//! [`records`] holds the retained values and their byte accounting.
//! [`host_evidence`] holds the exhaustive app-event mapping.
//! [`archive`] holds the capture slot, lifetime, and budget policy.
//! This module is the facade the rest of the app imports.

mod archive;
mod host_evidence;
mod records;

pub use archive::CaptureArchive;
pub use host_evidence::{BatchFacts, HostBatchKind, HostBatchSummary, HostScope, summarize};
pub use records::{
    AppendOutcome, AttemptKey, AttemptOrigin, AttemptRecord, CaptureId, CaptureRecord,
    CaptureState, CaptureStatus, CaptureSummary, HostBatchId, InstrumentationFlags, LimitKind,
    LimitReport, MAX_ATTEMPTS, MAX_BATCHES, MAX_BYTES, MAX_CAPTURES, MAX_MUTATIONS, SheetRef,
    StartRefusal, StopReason, estimate_attempt_bytes, estimate_capture_bytes,
    estimate_host_summary_bytes,
};
