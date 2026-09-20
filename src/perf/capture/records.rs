//! The capture record vocabulary and its byte accounting.
//!
//! These are the values the archive retains and a view reads: attempt and
//! capture identities, one retained attempt, the stop and limit vocabulary,
//! the view summaries, and the retained-byte estimates the budget is charged
//! with. They carry no lifecycle policy — that lives in [`super::archive`].

use std::mem::size_of;

use serde::Serialize;

use crate::perf::MutationSample;
use iron_canvas_core::{
    DiagBlit, DiagChangedCell, DiagFetchRequest, DiagGeometry, DiagRevealedStrip, DiagSegment,
    DiagSourceRange, FrameDiagnostics, RowSpan,
};

use super::host_evidence::{HostBatchSummary, HostScope};

/// Identity of one paint attempt inside one canvas generation.
///
/// The engine's attempt sequence, never a host frame counter: a held attempt
/// gets its own key, and a retry names the held key it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttemptKey {
    pub generation: u64,
    pub attempt_seq: u64,
}

/// One host emit's identity, counted by [`crate::events::EventBus`].
pub type HostBatchId = u64;

/// Identity of one capture.
pub type CaptureId = u64;

/// Sheet identity captured with a record.
///
/// `name` is resolved at capture time, because a later rename or delete must
/// not rewrite history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetRef {
    pub id: u32,
    pub name: Option<String>,
}

impl SheetRef {
    pub fn new(id: u32, name: Option<String>) -> Self {
        Self { id, name }
    }
}

/// Where one paint attempt came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AttemptOrigin {
    /// A host action or a repaint request reached the canvas.
    Live,
    /// The recorder forced a baseline paint. A tool action, not a host-input
    /// response, so it claims no host batches.
    ForcedBaseline,
}

/// One retained paint attempt.
#[derive(Clone, Debug, PartialEq)]
pub struct AttemptRecord {
    pub key: AttemptKey,
    pub captured_at_ms: f64,
    /// Duration of the `render_pending()` call that produced this attempt.
    /// `None` for an attempt the host did not time.
    pub render_call_ms: Option<f64>,
    pub origin: AttemptOrigin,
    /// This attempt retries that held attempt. Independent of `batch_ids`.
    pub retry_of: Option<AttemptKey>,
    /// Host batches claimed for this attempt, in order. Empty means no host
    /// batch preceded it: a resize, a font load, or a teardown paint.
    pub batch_ids: Vec<HostBatchId>,
    /// Sheet name resolved when the record was taken.
    pub sheet_name: Option<String>,
    /// Always present. The snapshot accessor supplies it.
    pub backing_size: (u32, u32),
    pub diagnostics: FrameDiagnostics,
}

/// Why a capture stopped.
///
/// Externally tagged: `"finished"` for a unit reason, and
/// `{"limitReached":"attempts"}` for the one that carries a limit. An internal
/// tag would collide with [`LimitKind`]'s own tag on the same object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    /// The operator finished it.
    Finished,
    /// A count limit or the byte budget applied.
    LimitReached(LimitKind),
    /// A new workbook generation took the canvas.
    GenerationEnded,
    /// An `.icr` recording was loaded for playback.
    PlaybackStarted,
}

impl StopReason {
    pub fn label(self) -> String {
        match self {
            Self::Finished => "finished".to_owned(),
            Self::LimitReached(kind) => format!("limit reached ({})", kind.label()),
            Self::GenerationEnded => "workbook replaced".to_owned(),
            Self::PlaybackStarted => "playback started".to_owned(),
        }
    }
}

/// Which limit stopped a capture. Serializes as a plain string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitKind {
    Attempts,
    Batches,
    Mutations,
    Bytes,
}

impl LimitKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Attempts => "attempts",
            Self::Batches => "host events",
            Self::Mutations => "mutations",
            Self::Bytes => "retained bytes",
        }
    }
}

/// Result of offering one record to the active capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    /// The capture already holds this `(generation, attempt_seq)`.
    Duplicate,
    /// The record was not retained. `reason` is the capture's stop reason:
    /// either the one just applied, or the reason it was already over.
    Rejected(StopReason),
}

/// Why a capture could not start or resume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartRefusal {
    /// Playback owns the canvas. Decided by the capture effect, which holds
    /// the playback state; the store has no other need for it.
    PlaybackActive,
    /// The archive already holds [`MAX_CAPTURES`] captures.
    CaptureLimitReached,
    CaptureActive,
    ByteBudgetExceeded,
}

impl StartRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::PlaybackActive => "playback is active",
            Self::CaptureLimitReached => "capture limit reached - delete one first",
            Self::CaptureActive => "a capture is already active",
            Self::ByteBudgetExceeded => "capture header exceeds the retained byte budget",
        }
    }
}

/// What the capture's own instrumentation recorded while it ran.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstrumentationFlags {
    /// True when a paint recording overlapped any part of this capture.
    /// Seeded when the capture starts during a recording, and never cleared.
    pub paint_recording: bool,
    /// Forced recorder baselines this capture retained.
    pub forced_baselines: usize,
}

/// Live or retained state of the capture slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    /// No capture. Records are not accepted.
    Idle,
    /// A capture was published and the canvas has not confirmed the flag yet.
    Starting,
    Capturing(CaptureId),
    Paused(CaptureId),
}

impl CaptureState {
    pub fn id(self) -> Option<CaptureId> {
        match self {
            Self::Idle | Self::Starting => None,
            Self::Capturing(id) | Self::Paused(id) => Some(id),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Capturing(_) => "capturing",
            Self::Paused(_) => "paused",
        }
    }
}

/// One retained capture.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureRecord {
    pub id: CaptureId,
    pub name: String,
    pub generation: u64,
    pub started_at_ms: f64,
    pub completed_at_ms: Option<f64>,
    /// Closed pause intervals.
    pub paused: Vec<(f64, f64)>,
    /// Start of the current pause. Kept in the record so a historical export
    /// can distinguish a paused capture from a running one.
    pub pause_started_at_ms: Option<f64>,
    pub stop_reason: Option<StopReason>,
    pub instrumentation: InstrumentationFlags,
    /// Informational roster taken at capture start. Never used to resolve an
    /// address; every record carries its own [`SheetRef`].
    pub sheet_names: Vec<(u32, String)>,
    pub attempts: Vec<AttemptRecord>,
    /// Host event summaries, ordered by `batch_id` ascending — the bus's
    /// monotonic counter appends them in order, so a lookup is a binary
    /// search rather than a scan. [`CaptureRecord::batch_summaries`] is the
    /// only reader that relies on the order.
    pub batches: Vec<HostBatchSummary>,
    pub mutations: Vec<MutationSample>,
    /// A limit stopped this capture before the operator finished it.
    pub truncated: bool,
    /// Records refused by the limit that stopped the capture.
    pub rejected_records: usize,
}

impl CaptureRecord {
    /// Duration from the start to `completed_at_ms`, or to `observed_at_ms`
    /// while the capture is still running. Includes pauses.
    pub fn wall_ms(&self, observed_at_ms: f64) -> f64 {
        self.completed_at_ms.unwrap_or(observed_at_ms) - self.started_at_ms
    }

    /// Every summary of one host batch, in emit order.
    ///
    /// Binary search over the `batch_id`-ordered `batches`, so an attempt's
    /// scope lookup costs `O(log n)` rather than a scan of every retained
    /// host event.
    pub fn batch_summaries(&self, batch_id: HostBatchId) -> &[HostBatchSummary] {
        let start = self
            .batches
            .partition_point(|summary| summary.batch_id < batch_id);
        let end = self
            .batches
            .partition_point(|summary| summary.batch_id <= batch_id);
        &self.batches[start..end]
    }
}

/// The frozen limits. Every one of them stops the capture when it applies.
pub const MAX_ATTEMPTS: usize = 500;
pub const MAX_BATCHES: usize = 5_000;
pub const MAX_MUTATIONS: usize = 2_000;
pub const MAX_CAPTURES: usize = 5;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Limit set and current retention, exported with every capture so a reader
/// can interpret a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitReport {
    pub max_attempts: usize,
    pub max_batches: usize,
    pub max_mutations: usize,
    pub max_captures: usize,
    pub max_bytes: usize,
    /// Retained bytes across the whole archive, when this report was taken.
    pub retained_bytes: usize,
    /// Captures the archive holds, when this report was taken.
    pub retained_captures: usize,
}

/// Estimated retained bytes of one attempt.
///
/// Walks the vectors and strings of the record. Never serializes a snapshot to
/// measure it — the estimate runs on the render path.
pub fn estimate_attempt_bytes(record: &AttemptRecord) -> usize {
    let diagnostics = &record.diagnostics;
    let mut bytes = size_of::<AttemptRecord>()
        + record.batch_ids.len() * size_of::<HostBatchId>()
        + record.sheet_name.as_ref().map_or(0, String::len)
        + diagnostics.fetch.requests.len() * size_of::<DiagFetchRequest>()
        + diagnostics.repaint.changed_rows.len() * size_of::<RowSpan>()
        + diagnostics.repaint.changed_cells.len() * size_of::<DiagChangedCell>()
        + diagnostics.repaint.source_ranges.len() * size_of::<DiagSourceRange>();
    if let Some(geometry) = &diagnostics.geometry {
        bytes += size_of::<DiagGeometry>() + geometry.segments.len() * size_of::<DiagSegment>();
    }
    if let Some(blit) = &diagnostics.blit {
        bytes += size_of::<DiagBlit>() + blit.revealed.len() * size_of::<DiagRevealedStrip>();
    }
    bytes
}

/// Estimated retained bytes of one host summary, including its sheet name.
pub fn estimate_host_summary_bytes(summary: &HostBatchSummary) -> usize {
    size_of::<HostBatchSummary>()
        + summary
            .sheet
            .as_ref()
            .and_then(|sheet| sheet.name.as_ref())
            .map_or(0, String::len)
        + match &summary.scope {
            Some(HostScope::Sheets { affected }) => affected.len() * size_of::<u32>(),
            _ => 0,
        }
}

/// Estimated retained bytes of the capture header: the record itself and the
/// roster resolved at its start.
pub fn estimate_capture_bytes(record: &CaptureRecord) -> usize {
    size_of::<CaptureRecord>()
        + record.name.len()
        + record
            .sheet_names
            .iter()
            .map(|(_, name)| size_of::<(u32, String)>() + name.len())
            .sum::<usize>()
        + record.paused.len() * size_of::<(f64, f64)>()
}

/// Facts a view needs about the capture slot, derived from the archive so two
/// signals can never disagree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureStatus {
    pub state: CaptureState,
    /// The capture the views show: the selection, else the active capture,
    /// else the newest retained one.
    pub selected: Option<CaptureId>,
    /// Attempts, host summaries, and mutation samples of that capture.
    pub attempts: usize,
    pub batches: usize,
    pub mutations: usize,
    pub retained_bytes: usize,
    pub retained_captures: usize,
    pub stop_reason: Option<StopReason>,
}

impl CaptureStatus {
    /// Nothing captured, nothing selected.
    pub fn idle() -> Self {
        Self {
            state: CaptureState::Idle,
            selected: None,
            attempts: 0,
            batches: 0,
            mutations: 0,
            retained_bytes: 0,
            retained_captures: 0,
            stop_reason: None,
        }
    }
}

/// One retained capture, for the inspector's capture selector.
///
/// A view needs the identity, the label, and whether the capture is the one
/// still in the live slot. The records themselves stay in the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSummary {
    pub id: CaptureId,
    pub name: String,
    pub attempts: usize,
    /// The capture in the live slot: running, starting, or paused.
    pub active: bool,
}
