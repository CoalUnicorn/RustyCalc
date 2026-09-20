//! Immutable capture records and the pure archive that retains them.
//!
//! Every rule about capture lifetime, admission, and the retained-byte budget
//! lives here as plain functions over `&mut CaptureArchive`. The signal layer
//! ([`super::PerfStore`]) owns the clock and the reactivity; this module owns
//! the behaviour, so the rules are testable without a browser and without a
//! canvas.
//!
//! Every limit *stops* the capture. No path drops a record and continues: a
//! capture that hit a limit says so, keeps the prefix it already retained, and
//! is marked truncated.

use std::mem::size_of;

use serde::Serialize;

use crate::coord::{CellAddress, SheetRange};
use crate::events::{
    ContentEvent, FormatEvent, HeaderChange, NavigationEvent, SpreadsheetEvent, StructureEvent,
    ThemeEvent,
};
use crate::perf::MutationSample;
use iron_canvas_core::RCRange;
use iron_canvas_core::RowSpan;
use iron_canvas_core::renderer::diag::{
    DiagBlit, DiagChangedCell, DiagFetchRequest, DiagGeometry, DiagRevealedStrip, DiagSegment,
    DiagSourceRange, FrameDiagnostics,
};

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

/// Category of one host event, mirroring the event bus categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HostBatchKind {
    Content,
    Format,
    Structure,
    Navigation,
    Theme,
}

impl HostBatchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Format => "format",
            Self::Structure => "structure",
            Self::Navigation => "navigation",
            Self::Theme => "theme",
        }
    }
}

/// What one host event pointed at.
///
/// Coordinates stay numeric and stay in the app's coordinate types. A1 text is
/// a view concern. Nothing here copies a value, a formula, a color list, or a
/// locale string — the scope and the kind are the whole record.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HostScope {
    Cell(CellAddress),
    Range(SheetRange),
    Viewport {
        sheet: u32,
        top_row: i32,
        left_col: i32,
    },
    Sheet {
        sheet: u32,
    },
    Sheets {
        affected: Vec<u32>,
    },
    Layout {
        sheet: u32,
        row: Option<i32>,
        col: Option<i32>,
    },
    Header(HeaderChange),
    RowMoved {
        sheet: u32,
        from_row: i32,
        to_row: i32,
    },
    ColumnMoved {
        sheet: u32,
        from_col: i32,
        to_col: i32,
    },
    /// A color list collapsed to its length.
    Colors {
        count: usize,
    },
}

impl HostScope {
    /// The addressable extent of this scope in canvas coordinates.
    ///
    /// `None` for a scope that names no rectangle: a viewport, a whole sheet,
    /// a set of sheets, a layout change, a header change, a moved row or
    /// column, or a colour list. Coordinates stay numeric — A1 text is a view
    /// concern.
    pub fn range(&self) -> Option<RCRange> {
        match self {
            Self::Cell(cell) => Some(RCRange {
                r1: cell.row,
                c1: cell.column,
                r2: cell.row,
                c2: cell.column,
            }),
            Self::Range(range) => Some(RCRange {
                r1: range.area.r1,
                c1: range.area.c1,
                r2: range.area.r2,
                c2: range.area.c2,
            }),
            _ => None,
        }
    }
}

/// The facts one host event contributes to a batch summary.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchFacts {
    pub kind: HostBatchKind,
    /// `None` when the event has no addressable target.
    pub scope: Option<HostScope>,
    /// Sheet the event acted on, for name resolution at capture time.
    pub sheet: Option<u32>,
}

/// Summarize one host event.
///
/// One arm per [`SpreadsheetEvent`] variant with no wildcard arm, so adding a
/// variant fails the build here instead of silently recording `None`.
pub fn summarize(event: &SpreadsheetEvent) -> BatchFacts {
    let (kind, scope, sheet) = match event {
        SpreadsheetEvent::Content(event) => match event {
            ContentEvent::CellChanged { address, .. } => (
                HostBatchKind::Content,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            ContentEvent::RangeChanged { sheet_area } => (
                HostBatchKind::Content,
                Some(HostScope::Range(*sheet_area)),
                Some(sheet_area.sheet),
            ),
            ContentEvent::FormulaChanged { address } => (
                HostBatchKind::Content,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            ContentEvent::CalculationUpdated { affected_sheets } => (
                HostBatchKind::Content,
                Some(HostScope::Sheets {
                    affected: affected_sheets.clone(),
                }),
                None,
            ),
            ContentEvent::NamedRangesChanged => (HostBatchKind::Content, None, None),
        },
        SpreadsheetEvent::Format(event) => match event {
            FormatEvent::CellStyleChanged { address } => (
                HostBatchKind::Format,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            FormatEvent::RangeStyleChanged { area } => (
                HostBatchKind::Format,
                Some(HostScope::Range(*area)),
                Some(area.sheet),
            ),
            FormatEvent::LayoutChanged { sheet, col, row } => (
                HostBatchKind::Format,
                Some(HostScope::Layout {
                    sheet: *sheet,
                    row: *row,
                    col: *col,
                }),
                Some(*sheet),
            ),
            FormatEvent::RecentColorsUpdated { colors } => (
                HostBatchKind::Format,
                Some(HostScope::Colors {
                    count: colors.len(),
                }),
                None,
            ),
            FormatEvent::DocumentColorsChanged { colors } => (
                HostBatchKind::Format,
                Some(HostScope::Colors {
                    count: colors.len(),
                }),
                None,
            ),
            FormatEvent::ConditionalFormattingChanged { sheet } => (
                HostBatchKind::Format,
                Some(HostScope::Sheet { sheet: *sheet }),
                Some(*sheet),
            ),
        },
        SpreadsheetEvent::Structure(event) => match event {
            StructureEvent::WorksheetAdded { sheet, .. }
            | StructureEvent::WorksheetDeleted { sheet }
            | StructureEvent::WorksheetRenamed { sheet, .. }
            | StructureEvent::WorksheetHidden { sheet }
            | StructureEvent::WorksheetUnhidden { sheet, .. } => (
                HostBatchKind::Structure,
                Some(HostScope::Sheet { sheet: *sheet }),
                Some(*sheet),
            ),
            StructureEvent::WorksheetsReordered => (HostBatchKind::Structure, None, None),
            StructureEvent::StructureChanged(change) => (
                HostBatchKind::Structure,
                Some(HostScope::Header(change.clone())),
                Some(change.sheet),
            ),
            StructureEvent::ColumnMoved {
                sheet,
                from_col,
                to_col,
            } => (
                HostBatchKind::Structure,
                Some(HostScope::ColumnMoved {
                    sheet: *sheet,
                    from_col: *from_col,
                    to_col: *to_col,
                }),
                Some(*sheet),
            ),
            StructureEvent::RowMoved {
                sheet,
                from_row,
                to_row,
            } => (
                HostBatchKind::Structure,
                Some(HostScope::RowMoved {
                    sheet: *sheet,
                    from_row: *from_row,
                    to_row: *to_row,
                }),
                Some(*sheet),
            ),
            StructureEvent::DocumentReset => (HostBatchKind::Structure, None, None),
        },
        SpreadsheetEvent::Navigation(event) => match event {
            NavigationEvent::SelectionChanged { address } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            NavigationEvent::SelectionRangeChanged { sheet_area } => (
                HostBatchKind::Navigation,
                Some(HostScope::Range(*sheet_area)),
                Some(sheet_area.sheet),
            ),
            NavigationEvent::ViewportScrolled {
                sheet,
                top_row,
                left_col,
            } => (
                HostBatchKind::Navigation,
                Some(HostScope::Viewport {
                    sheet: *sheet,
                    top_row: *top_row,
                    left_col: *left_col,
                }),
                Some(*sheet),
            ),
            NavigationEvent::ActiveSheetChanged {
                from_sheet,
                to_sheet,
            } => (
                HostBatchKind::Navigation,
                Some(HostScope::Sheets {
                    affected: vec![*from_sheet, *to_sheet],
                }),
                Some(*to_sheet),
            ),
            NavigationEvent::EditingStarted { address } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
            NavigationEvent::EditingEnded { address, .. } => (
                HostBatchKind::Navigation,
                Some(HostScope::Cell(*address)),
                Some(address.sheet),
            ),
        },
        SpreadsheetEvent::Theme(event) => match event {
            ThemeEvent::ThemeToggled { .. }
            | ThemeEvent::PaletteUpdated
            | ThemeEvent::LocaleChanged { .. } => (HostBatchKind::Theme, None, None),
        },
    };
    BatchFacts { kind, scope, sheet }
}

/// One retained host event, with the sheet identity resolved at capture time.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostBatchSummary {
    pub batch_id: HostBatchId,
    /// Position of this event inside its batch.
    pub index: u32,
    pub at_ms: f64,
    pub kind: HostBatchKind,
    pub scope: Option<HostScope>,
    /// Sheet identity for a scoped event, resolved at capture time.
    pub sheet: Option<SheetRef>,
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

/// The capture slot, the retained captures, and the read-side state a view
/// needs. Holds no signal: [`super::PerfStore`] wraps this.
#[derive(Debug)]
pub struct CaptureArchive {
    next_capture_id: CaptureId,
    generation: u64,
    lifecycle: Lifecycle,
    archived: Vec<CaptureRecord>,
    selected: Option<CaptureId>,
    retained_bytes: usize,
    /// Bytes charged to each retained capture, so a delete releases exactly
    /// what that capture was charged.
    charges: Vec<(CaptureId, usize)>,
    /// Bytes charged to the capture in the active or starting slot.
    slot_charge: usize,
    /// Host batches observed since the last claim.
    pending_batches: Vec<HostBatchId>,
    /// The attempt a retry should name.
    last_held: Option<AttemptKey>,
}

#[derive(Debug)]
enum Lifecycle {
    Idle,
    /// A capture whose canvas flag the coordinator has not confirmed yet.
    Starting {
        record: Box<CaptureRecord>,
    },
    Active {
        record: Box<CaptureRecord>,
    },
}

impl Default for CaptureArchive {
    fn default() -> Self {
        Self::new()
    }
}

impl CaptureArchive {
    pub fn new() -> Self {
        Self {
            next_capture_id: 1,
            generation: 0,
            lifecycle: Lifecycle::Idle,
            archived: Vec::new(),
            selected: None,
            retained_bytes: 0,
            charges: Vec::new(),
            slot_charge: 0,
            pending_batches: Vec::new(),
            last_held: None,
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub fn limit_report(&self) -> LimitReport {
        LimitReport {
            max_attempts: MAX_ATTEMPTS,
            max_batches: MAX_BATCHES,
            max_mutations: MAX_MUTATIONS,
            max_captures: MAX_CAPTURES,
            max_bytes: MAX_BYTES,
            retained_bytes: self.retained_bytes,
            retained_captures: self.retained_captures(),
        }
    }

    /// The capture the views show: the selection when it still exists, else
    /// the active capture, else the newest retained one.
    pub fn selected(&self) -> Option<CaptureId> {
        self.selected
            .filter(|id| self.contains(*id))
            .or_else(|| self.lifecycle.id())
            .or_else(|| self.archived.last().map(|capture| capture.id))
    }

    pub fn state(&self) -> CaptureState {
        match &self.lifecycle {
            Lifecycle::Idle => CaptureState::Idle,
            Lifecycle::Starting { .. } => CaptureState::Starting,
            Lifecycle::Active { record } => match record.pause_started_at_ms {
                None => CaptureState::Capturing(record.id),
                Some(_) => CaptureState::Paused(record.id),
            },
        }
    }

    /// Captures held by the archive, including the one in the active slot.
    pub fn retained_captures(&self) -> usize {
        self.archived.len() + usize::from(self.lifecycle.record().is_some())
    }

    pub fn active_id(&self) -> Option<CaptureId> {
        self.lifecycle.id()
    }

    pub fn with_capture<R>(&self, id: CaptureId, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        if let Some(record) = self.lifecycle.record().filter(|record| record.id == id) {
            return Some(f(record));
        }
        self.archived.iter().find(|capture| capture.id == id).map(f)
    }

    pub fn with_active<R>(&self, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        self.lifecycle.record().map(f)
    }

    /// One row per retained capture, for the inspector's capture selector.
    ///
    /// The active capture comes first, then the archived captures newest
    /// first: the list reads as "what is running, then what I can look back
    /// at".
    pub fn capture_summaries(&self) -> Vec<CaptureSummary> {
        let mut out = Vec::with_capacity(self.retained_captures());
        if let Some(record) = self.lifecycle.record() {
            out.push(CaptureSummary {
                id: record.id,
                name: record.name.clone(),
                attempts: record.attempts.len(),
                active: true,
            });
        }
        out.extend(self.archived.iter().rev().map(|record| CaptureSummary {
            id: record.id,
            name: record.name.clone(),
            attempts: record.attempts.len(),
            active: false,
        }));
        out
    }

    /// Read the selected capture, or the newest one when nothing is selected.
    pub fn with_selected<R>(&self, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        self.selected().and_then(|id| self.with_capture(id, f))
    }

    /// Select a retained capture. Never retargets the active slot.
    pub fn select(&mut self, id: CaptureId) -> bool {
        if !self.contains(id) {
            return false;
        }
        self.selected = Some(id);
        true
    }

    /// Publish the derived facts a view renders.
    pub fn status(&self) -> CaptureStatus {
        let selected = self.selected();
        let counts = self
            .with_selected(|capture| {
                (
                    capture.attempts.len(),
                    capture.batches.len(),
                    capture.mutations.len(),
                    capture.stop_reason,
                )
            })
            .unwrap_or((0, 0, 0, None));
        CaptureStatus {
            state: self.state(),
            selected,
            attempts: counts.0,
            batches: counts.1,
            mutations: counts.2,
            retained_bytes: self.retained_bytes,
            retained_captures: self.retained_captures(),
            stop_reason: counts.3,
        }
    }

    /// Admit a new capture. Refused while playback owns the canvas and while
    /// the archive is full.
    pub fn request_start(
        &mut self,
        playback_active: bool,
        started_at_ms: f64,
        sheet_names: Vec<(u32, String)>,
    ) -> Result<CaptureId, StartRefusal> {
        if playback_active {
            return Err(StartRefusal::PlaybackActive);
        }
        if !matches!(self.lifecycle, Lifecycle::Idle) {
            return Err(StartRefusal::CaptureActive);
        }
        if self.retained_captures() >= MAX_CAPTURES {
            return Err(StartRefusal::CaptureLimitReached);
        }
        let id = self.next_capture_id;
        let record = CaptureRecord {
            id,
            name: format!("capture-{id}"),
            generation: self.generation,
            started_at_ms,
            completed_at_ms: None,
            paused: Vec::new(),
            pause_started_at_ms: None,
            stop_reason: None,
            instrumentation: InstrumentationFlags::default(),
            sheet_names,
            attempts: Vec::new(),
            batches: Vec::new(),
            mutations: Vec::new(),
            truncated: false,
            rejected_records: 0,
        };
        let charge = estimate_capture_bytes(&record);
        if charge > MAX_BYTES.saturating_sub(self.retained_bytes) {
            return Err(StartRefusal::ByteBudgetExceeded);
        }
        self.next_capture_id = self.next_capture_id.wrapping_add(1);
        self.slot_charge = charge;
        self.retained_bytes += self.slot_charge;
        self.pending_batches.clear();
        self.last_held = None;
        self.lifecycle = Lifecycle::Starting {
            record: Box::new(record),
        };
        Ok(id)
    }

    /// The canvas accepted the capture flag. Publish a capture that is still
    /// waiting to be published.
    ///
    /// A capture that is already `Active` is untouched: the coordinator calls
    /// this whenever it re-applies the flag, and resuming must not restart or
    /// drop the capture it already published.
    pub fn confirm_start(&mut self) {
        if !matches!(self.lifecycle, Lifecycle::Starting { .. }) {
            return;
        }
        if let Lifecycle::Starting { record } =
            std::mem::replace(&mut self.lifecycle, Lifecycle::Idle)
        {
            self.lifecycle = Lifecycle::Active { record };
        }
    }

    /// The canvas did not take the capture flag.
    ///
    /// An unpublished start is dropped — nothing was measured, so nothing is
    /// retained. A published capture is paused instead: it keeps every record,
    /// and the operator can resume once the canvas is back.
    pub fn fail_start(&mut self, at_ms: f64) {
        if !matches!(self.lifecycle, Lifecycle::Starting { .. }) {
            self.pause(at_ms);
            return;
        }
        if let Lifecycle::Starting { record } =
            std::mem::replace(&mut self.lifecycle, Lifecycle::Idle)
        {
            debug_assert_eq!(self.slot_charge, estimate_capture_bytes(&record));
            self.retained_bytes = self.retained_bytes.saturating_sub(self.slot_charge);
            self.slot_charge = 0;
        }
    }

    /// Stop accepting records but keep everything. Resuming closes the gap.
    pub fn pause(&mut self, at_ms: f64) {
        if !self.collecting() {
            return;
        }
        // Reserve the closed interval before opening it. Resume and finish
        // must be able to close a pause even when the archive is full.
        let bytes = size_of::<(f64, f64)>();
        if bytes > MAX_BYTES.saturating_sub(self.retained_bytes) {
            self.stop_at_limit(LimitKind::Bytes, at_ms, 1);
            return;
        }
        self.retained_bytes += bytes;
        self.slot_charge += bytes;
        if let Lifecycle::Active { record } = &mut self.lifecycle {
            record.pause_started_at_ms = Some(at_ms);
            // A batch that arrived while paused is not claimed: it preceded no
            // attempt the capture will keep.
            self.pending_batches.clear();
            self.last_held = None;
        }
    }

    /// Resume a paused capture. Refused while playback owns the canvas. A
    /// capture that is not paused is unchanged.
    pub fn resume(&mut self, playback_active: bool, at_ms: f64) -> Result<(), StartRefusal> {
        if playback_active {
            return Err(StartRefusal::PlaybackActive);
        }
        if let Lifecycle::Active { record } = &mut self.lifecycle
            && let Some(started) = record.pause_started_at_ms.take()
        {
            record.paused.push((started, at_ms));
        }
        Ok(())
    }

    /// Close the active capture into the archive.
    pub fn finish(&mut self, reason: StopReason, at_ms: f64) {
        self.pending_batches.clear();
        self.last_held = None;
        match std::mem::replace(&mut self.lifecycle, Lifecycle::Idle) {
            Lifecycle::Idle => {}
            Lifecycle::Starting { record } => {
                // Never reached the canvas: nothing was measured, so nothing is
                // retained.
                self.retained_bytes = self.retained_bytes.saturating_sub(self.slot_charge);
                self.slot_charge = 0;
                debug_assert!(record.attempts.is_empty());
            }
            Lifecycle::Active { mut record } => {
                if let Some(started) = record.pause_started_at_ms.take() {
                    record.paused.push((started, at_ms));
                }
                record.completed_at_ms = Some(at_ms);
                record.stop_reason = Some(reason);
                record.truncated = matches!(reason, StopReason::LimitReached(_));
                let id = record.id;
                let charge = self.slot_charge;
                self.slot_charge = 0;
                self.archived.push(*record);
                self.charges.push((id, charge));
            }
        }
    }

    /// A generation change: close the active capture and open a new one.
    pub fn end_generation(&mut self, at_ms: f64) {
        self.finish(StopReason::GenerationEnded, at_ms);
        self.generation = self.generation.wrapping_add(1);
    }

    /// Note that a paint recording overlapped this capture. Seeds the flag and
    /// never clears it.
    pub fn note_paint_recording(&mut self) {
        if let Some(record) = self.lifecycle.record_mut() {
            record.instrumentation.paint_recording = true;
        }
    }

    /// Offer one attempt to the active capture.
    pub fn append_attempt(&mut self, record: AttemptRecord, at_ms: f64) -> AppendOutcome {
        if !self.collecting() {
            return AppendOutcome::Rejected(
                self.last_stop_reason().unwrap_or(StopReason::Finished),
            );
        }
        let Lifecycle::Active { record: active } = &mut self.lifecycle else {
            return AppendOutcome::Rejected(
                self.last_stop_reason().unwrap_or(StopReason::Finished),
            );
        };
        if active
            .attempts
            .iter()
            .any(|attempt| attempt.key == record.key)
        {
            return AppendOutcome::Duplicate;
        }
        if active.attempts.len() >= MAX_ATTEMPTS {
            self.stop_at_limit(LimitKind::Attempts, at_ms, 1);
            return AppendOutcome::Rejected(StopReason::LimitReached(LimitKind::Attempts));
        }
        let bytes = estimate_attempt_bytes(&record);
        if self.retained_bytes + bytes > MAX_BYTES {
            self.stop_at_limit(LimitKind::Bytes, at_ms, 1);
            return AppendOutcome::Rejected(StopReason::LimitReached(LimitKind::Bytes));
        }
        let forced_baseline = record.origin == AttemptOrigin::ForcedBaseline;
        if let Lifecycle::Active { record: active, .. } = &mut self.lifecycle {
            if forced_baseline {
                active.instrumentation.forced_baselines += 1;
            }
            active.attempts.push(record);
        }
        self.retained_bytes += bytes;
        self.slot_charge += bytes;
        AppendOutcome::Appended
    }

    /// Note one host emit. One summary per event, ordered inside the batch.
    ///
    /// A batch is admitted whole or not at all: it is one host input, and a
    /// half-recorded batch would misstate what preceded the attempt.
    /// `sheet_name` must be a pure lookup — it runs against the model roster.
    pub fn note_batch(
        &mut self,
        batch_id: HostBatchId,
        events: &[SpreadsheetEvent],
        at_ms: f64,
        sheet_name: impl Fn(u32) -> Option<String>,
    ) {
        if events.is_empty() || !self.collecting() {
            return;
        }
        let retained_batches = match &self.lifecycle {
            Lifecycle::Active { record, .. } => record.batches.len(),
            _ => return,
        };
        let mut summaries = Vec::with_capacity(events.len());
        let mut bytes = 0;
        for (index, event) in events.iter().enumerate() {
            let facts = summarize(event);
            let summary = HostBatchSummary {
                batch_id,
                index: index as u32,
                at_ms,
                kind: facts.kind,
                scope: facts.scope,
                sheet: facts.sheet.map(|id| {
                    // Deletion has already changed the roster. The same index
                    // may now name a different sheet, so do not resolve it.
                    let name = if matches!(
                        event,
                        SpreadsheetEvent::Structure(StructureEvent::WorksheetDeleted { .. })
                    ) {
                        None
                    } else {
                        sheet_name(id)
                    };
                    SheetRef::new(id, name)
                }),
            };
            bytes += estimate_host_summary_bytes(&summary);
            summaries.push(summary);
        }
        if retained_batches + summaries.len() > MAX_BATCHES {
            self.stop_at_limit(LimitKind::Batches, at_ms, summaries.len());
            return;
        }
        if self.retained_bytes + bytes > MAX_BYTES {
            self.stop_at_limit(LimitKind::Bytes, at_ms, summaries.len());
            return;
        }
        if let Lifecycle::Active { record, .. } = &mut self.lifecycle {
            record.batches.extend(summaries);
        }
        self.retained_bytes += bytes;
        self.slot_charge += bytes;
        self.pending_batches.push(batch_id);
    }

    /// Note one completed model mutation.
    pub fn note_mutation(&mut self, sample: MutationSample, at_ms: f64) {
        if !self.collecting() {
            return;
        }
        let retained = match &self.lifecycle {
            Lifecycle::Active { record, .. } => record.mutations.len(),
            _ => return,
        };
        let bytes = size_of::<MutationSample>();
        if retained >= MAX_MUTATIONS {
            self.stop_at_limit(LimitKind::Mutations, at_ms, 1);
            return;
        }
        if self.retained_bytes + bytes > MAX_BYTES {
            self.stop_at_limit(LimitKind::Bytes, at_ms, 1);
            return;
        }
        if let Lifecycle::Active { record, .. } = &mut self.lifecycle {
            record.mutations.push(sample);
        }
        self.retained_bytes += bytes;
        self.slot_charge += bytes;
    }

    /// Take the host batches observed since the last claim.
    pub fn claim_pending_batches(&mut self) -> Vec<HostBatchId> {
        std::mem::take(&mut self.pending_batches)
    }

    /// The attempt a retry should name.
    pub fn retry_link(&self) -> Option<AttemptKey> {
        self.last_held
    }

    /// Remember, or forget, the held attempt a retry names. A capture that is
    /// not running keeps no memory: the retry link only matters inside one
    /// capture.
    pub fn remember_held(&mut self, key: AttemptKey, held: bool) {
        if !self.collecting() {
            return;
        }
        self.last_held = held.then_some(key);
    }

    /// Delete a retained capture and release exactly what it was charged.
    pub fn delete_capture(&mut self, id: CaptureId) -> bool {
        let Some(index) = self.archived.iter().position(|capture| capture.id == id) else {
            return false;
        };
        self.archived.remove(index);
        let charge = self
            .charges
            .iter()
            .position(|(charged_id, _)| *charged_id == id)
            .map(|index| self.charges.remove(index).1)
            .unwrap_or(0);
        self.retained_bytes = self.retained_bytes.saturating_sub(charge);
        if self.selected == Some(id) {
            self.selected = None;
        }
        true
    }

    fn contains(&self, id: CaptureId) -> bool {
        self.lifecycle.id() == Some(id) || self.archived.iter().any(|capture| capture.id == id)
    }

    /// Whether the capture is collecting right now. A paused capture holds an
    /// open pause interval, so this is not the same question as "is a capture
    /// active": a batch or a mutation that arrives while paused precedes no
    /// attempt the capture will keep, and attributing it to the first attempt
    /// after a resume would invent provenance.
    fn collecting(&self) -> bool {
        matches!(
            &self.lifecycle,
            Lifecycle::Active { record } if record.pause_started_at_ms.is_none()
        )
    }

    fn last_stop_reason(&self) -> Option<StopReason> {
        self.archived.last().and_then(|capture| capture.stop_reason)
    }

    /// Stop the active capture because a limit applied. The capture keeps the
    /// prefix it already holds and records what it refused.
    fn stop_at_limit(&mut self, kind: LimitKind, at_ms: f64, rejected: usize) {
        if let Lifecycle::Active { record, .. } = &mut self.lifecycle {
            record.rejected_records += rejected;
        }
        self.finish(StopReason::LimitReached(kind), at_ms);
    }
}

impl Lifecycle {
    fn id(&self) -> Option<CaptureId> {
        match self {
            Self::Idle => None,
            Self::Starting { record } | Self::Active { record, .. } => Some(record.id),
        }
    }

    fn record(&self) -> Option<&CaptureRecord> {
        match self {
            Self::Idle => None,
            Self::Starting { record } | Self::Active { record, .. } => Some(record),
        }
    }

    fn record_mut(&mut self) -> Option<&mut CaptureRecord> {
        match self {
            Self::Idle => None,
            Self::Starting { record } | Self::Active { record, .. } => Some(record),
        }
    }
}
