//! The capture slot and the pure archive that retains captures.
//!
//! Every rule about capture lifetime, admission, and the retained-byte budget
//! lives here as plain functions over `&mut CaptureArchive`. The signal layer
//! ([`crate::perf::PerfStore`]) owns the clock and the reactivity; this module owns
//! the behaviour, so the rules are testable without a browser and without a
//! canvas.
//!
//! Every limit *stops* the capture. No path drops a record and continues: a
//! capture that hit a limit says so, keeps the prefix it already retained, and
//! is marked truncated.

use std::mem::size_of;

use crate::events::{SpreadsheetEvent, StructureEvent};
use crate::perf::MutationSample;

use super::host_evidence::{HostBatchSummary, summarize};
use super::records::{
    AppendOutcome, AttemptKey, AttemptOrigin, AttemptRecord, CaptureId, CaptureRecord,
    CaptureState, CaptureStatus, CaptureSummary, HostBatchId, InstrumentationFlags, LimitKind,
    LimitReport, MAX_ATTEMPTS, MAX_BATCHES, MAX_BYTES, MAX_CAPTURES, MAX_MUTATIONS, SheetRef,
    StartRefusal, StopReason, estimate_attempt_bytes, estimate_capture_bytes,
    estimate_host_summary_bytes,
};

/// The capture slot, the retained captures, and the read-side state a view
/// needs. Holds no signal: [`crate::perf::PerfStore`] wraps this.
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
