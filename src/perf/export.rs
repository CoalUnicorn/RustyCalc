//! JSON export of a retained capture.
//!
//! The core `FrameDiagnostics` type does not implement `Serialize` by design:
//! the web projection owns serialization. This module therefore embeds each
//! stored snapshot as a JSON **object** through the web helper. It never
//! serializes the core type directly, and it never stores an escaped JSON
//! string.
//!
//! Serialization happens here and nowhere else. The render loop never
//! serializes.

use serde::Serialize;

use super::capture::{
    AttemptKey, AttemptOrigin, AttemptRecord, CaptureId, CaptureRecord, HostBatchId,
    HostBatchSummary, InstrumentationFlags, LimitReport, StopReason,
};
use super::timing::MutationSample;

/// Envelope format version. Independent of the diagnostics schema version it
/// carries.
pub const CAPTURE_ENVELOPE_VERSION: u32 = 1;

/// Why an export could not be built.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("attempt {generation}/{attempt_seq} has no projectable diagnostics")]
    Projection { generation: u64, attempt_seq: u64 },
    #[error("capture JSON serialization failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Export-wide facts: format version, limits, and the retention accounting of
/// the archive the capture came from.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureEnvelopeWire {
    pub version: u32,
    pub observed_at_ms: f64,
    /// Diagnostics schema version of the embedded snapshots. Zero when the
    /// capture holds no attempt.
    pub diagnostics_schema_version: u8,
    pub limits: LimitReport,
    pub capture: CaptureWire,
}

/// One capture, with its diagnostics embedded as objects.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureWire {
    pub id: CaptureId,
    pub name: String,
    pub generation: u64,
    pub started_at_ms: f64,
    pub completed_at_ms: Option<f64>,
    pub paused: Vec<(f64, f64)>,
    pub pause_started_at_ms: Option<f64>,
    pub stop_reason: Option<StopReason>,
    pub instrumentation: InstrumentationFlags,
    pub sheet_names: Vec<(u32, String)>,
    pub truncated: bool,
    pub rejected_records: usize,
    pub attempts: Vec<AttemptWire>,
    pub batches: Vec<HostBatchSummary>,
    pub mutations: Vec<MutationSample>,
}

/// One attempt. `backing_size` is mandatory on both sides, so the embedded
/// geometry can never disagree with the snapshot it came from.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptWire {
    pub key: AttemptKey,
    pub captured_at_ms: f64,
    pub render_call_ms: Option<f64>,
    pub origin: AttemptOrigin,
    pub retry_of: Option<AttemptKey>,
    pub batch_ids: Vec<HostBatchId>,
    pub sheet_name: Option<String>,
    pub backing_size: (u32, u32),
    /// A JSON object from `frame_diagnostics_value`.
    pub diagnostics: serde_json::Value,
}

/// Project one stored snapshot through the web helper.
fn attempt_wire(record: &AttemptRecord) -> Result<AttemptWire, ExportError> {
    let snapshot = iron_canvas_web::CanvasFrameSnapshot {
        diagnostics: record.diagnostics.clone(),
        backing_size: record.backing_size,
    };
    let diagnostics =
        iron_canvas_web::frame_diagnostics_value(&snapshot).ok_or(ExportError::Projection {
            generation: record.key.generation,
            attempt_seq: record.key.attempt_seq,
        })?;
    Ok(AttemptWire {
        key: record.key,
        captured_at_ms: record.captured_at_ms,
        render_call_ms: record.render_call_ms,
        origin: record.origin,
        retry_of: record.retry_of,
        batch_ids: record.batch_ids.clone(),
        sheet_name: record.sheet_name.clone(),
        backing_size: record.backing_size,
        diagnostics,
    })
}

/// Export a whole capture. `limits` and the retention totals describe the
/// archive the capture came from, so a reader can interpret a stop.
pub fn capture_json(
    capture: &CaptureRecord,
    limits: &LimitReport,
    observed_at_ms: f64,
) -> Result<String, ExportError> {
    let attempts = capture
        .attempts
        .iter()
        .map(attempt_wire)
        .collect::<Result<Vec<_>, _>>()?;
    let envelope = CaptureEnvelopeWire {
        version: CAPTURE_ENVELOPE_VERSION,
        observed_at_ms,
        diagnostics_schema_version: capture
            .attempts
            .first()
            .map_or(0, |attempt| attempt.diagnostics.schema_version),
        limits: *limits,
        capture: CaptureWire {
            id: capture.id,
            name: capture.name.clone(),
            generation: capture.generation,
            started_at_ms: capture.started_at_ms,
            completed_at_ms: capture.completed_at_ms,
            paused: capture.paused.clone(),
            pause_started_at_ms: capture.pause_started_at_ms,
            stop_reason: capture.stop_reason,
            instrumentation: capture.instrumentation,
            sheet_names: capture.sheet_names.clone(),
            truncated: capture.truncated,
            rejected_records: capture.rejected_records,
            attempts,
            batches: capture.batches.clone(),
            mutations: capture.mutations.clone(),
        },
    };
    Ok(serde_json::to_string_pretty(&envelope)?)
}

/// Export one attempt on its own, for the copy action.
pub fn attempt_json(record: &AttemptRecord) -> Result<String, ExportError> {
    Ok(serde_json::to_string_pretty(&attempt_wire(record)?)?)
}
