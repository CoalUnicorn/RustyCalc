//! Diagnostic capture for dev builds: the published snapshot vocabulary,
//! the renderer-owned capture state, and the publication boundary.
//!
//! `FrameTrace` answers "which path painted this frame?" in one allocation-
//! free line. This module answers "why?" with a typed snapshot of the same
//! attempt: planned segments, renderer-owned fetch requests, the repaint
//! decision and its reason, the prepared/committed cache transition, blit
//! geometry, and painted row/cell counts.
//!
//! Layout:
//!
//! - `snapshot` is the published vocabulary — `FrameDiagnostics`, the
//!   `Diag*` mirrors, `DIAG_SCHEMA_VERSION`. It stays ungated, so the shape
//!   of a snapshot is nameable in every build, and a consumer never names
//!   renderer state to read an evidence fact. The crate-root re-exports
//!   carry the same names.
//! - `state` and `capture` are collection: the in-flight buffer, the private
//!   prepared-data reads, and cache sampling. Both compile only with the
//!   `dev-diagnostics` feature.
//!
//! Capture is a pure observer: nothing here re-runs classifiers, changes
//! planner outcomes, or touches committed cache state. All writes are
//! feature-gated (`dev-diagnostics`) and no-ops while `enabled` is false,
//! so disabled capture performs no allocations. Wall-clock reads belong to
//! the host; core never samples a clock here.
//!
//! The grid `RendererCore` owns one `DiagState`: an in-flight `capture`
//! buffer written during prepare/execute by the `capture` module, and a
//! `published` last snapshot moved there only by
//! `Orchestrator::finish_attempt` — so a held attempt can never surface
//! candidate layout or cache state as committed.

mod snapshot;

pub use snapshot::*;

#[cfg(feature = "dev-diagnostics")]
mod capture;
#[cfg(feature = "dev-diagnostics")]
mod state;

#[cfg(feature = "dev-diagnostics")]
pub(crate) use capture::distinct_rows;
#[cfg(feature = "dev-diagnostics")]
pub(crate) use state::{DiagCompletion, DiagState};
