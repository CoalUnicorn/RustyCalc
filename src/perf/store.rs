//! Signal layer over [`CaptureArchive`].
//!
//! The store owns the clock and the reactive facts; the archive owns every
//! rule. Views read the small derived signals (`state`, `status`,
//! `revision`); they never clone a capture into a signal.
//!
//! One thing the store deliberately does not do: touch the canvas. Any path
//! that changes the desired canvas capture flag bumps `sync_request`, and the
//! worksheet's capture Effect reacts. That keeps the canvas borrow out of
//! every callback and out of the observer.

use leptos::prelude::*;

use crate::events::SpreadsheetEvent;
use crate::perf::{MutationSample, now};

use super::capture::{
    AppendOutcome, AttemptKey, AttemptRecord, CaptureArchive, CaptureId, CaptureRecord,
    CaptureState, CaptureStatus, CaptureSummary, HostBatchId, LimitReport, StartRefusal,
    StopReason,
};

/// The capture store: signals only, so `AppState` stays `Copy`.
#[derive(Clone, Copy)]
pub struct PerfStore {
    archive: RwSignal<CaptureArchive>,
    state: RwSignal<CaptureState>,
    status: RwSignal<CaptureStatus>,
    revision: RwSignal<u64>,
    sync_request: RwSignal<u64>,
    /// Last refusal or failure, for the panel. Cleared by the next successful
    /// start.
    error: RwSignal<Option<String>>,
}

impl Default for PerfStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfStore {
    pub fn new() -> Self {
        let archive = RwSignal::new(CaptureArchive::new());
        Self {
            archive,
            state: RwSignal::new(CaptureState::Idle),
            status: RwSignal::new(CaptureStatus::idle()),
            revision: RwSignal::new(0),
            sync_request: RwSignal::new(0),
            error: RwSignal::new(None),
        }
    }

    /// Bumped on every change a view renders. A view that reads captures
    /// tracks this instead of the archive signal.
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn state(&self) -> CaptureState {
        self.state.get()
    }

    pub fn status(&self) -> CaptureStatus {
        self.status.get()
    }

    /// State read outside a reactive context: the render loop and the Effects
    /// call this so their reads do not subscribe.
    pub fn state_untracked(&self) -> CaptureState {
        self.state.get_untracked()
    }

    pub fn status_untracked(&self) -> CaptureStatus {
        self.status.get_untracked()
    }

    pub fn error(&self) -> Option<String> {
        self.error.get()
    }

    pub fn generation(&self) -> u64 {
        self.archive.with_untracked(CaptureArchive::generation)
    }

    pub fn retained_bytes(&self) -> usize {
        self.archive.with_untracked(CaptureArchive::retained_bytes)
    }

    pub fn limit_report(&self) -> LimitReport {
        self.archive.with_untracked(CaptureArchive::limit_report)
    }

    pub fn selected(&self) -> Option<CaptureId> {
        self.status.with_untracked(|status| status.selected)
    }

    pub fn with_capture<R>(&self, id: CaptureId, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        self.archive
            .with_untracked(|archive| archive.with_capture(id, f))
    }

    pub fn with_active<R>(&self, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        self.archive
            .with_untracked(|archive| archive.with_active(f))
    }

    pub fn with_selected<R>(&self, f: impl FnOnce(&CaptureRecord) -> R) -> Option<R> {
        self.archive
            .with_untracked(|archive| archive.with_selected(f))
    }

    /// One row per retained capture, for the inspector's capture selector.
    pub fn capture_summaries(&self) -> Vec<CaptureSummary> {
        self.archive
            .with_untracked(CaptureArchive::capture_summaries)
    }

    /// The newest attempt of the capture the views show.
    pub fn with_latest_attempt<R>(&self, f: impl FnOnce(&AttemptRecord) -> R) -> Option<R> {
        self.with_selected(|capture| capture.attempts.last().map(f))
            .flatten()
    }

    /// Start a capture. Refused while playback owns the canvas and when the
    /// archive already holds the capture limit.
    pub fn request_start(
        &self,
        playback_active: bool,
        sheet_names: Vec<(u32, String)>,
    ) -> Result<CaptureId, StartRefusal> {
        let at_ms = now();
        let outcome = self
            .archive
            .update_untracked(|archive| archive.request_start(playback_active, at_ms, sheet_names));
        if outcome.is_ok() {
            self.error.set(None);
        } else if let Err(refusal) = outcome {
            self.error.set(Some(refusal.label().to_owned()));
        }
        self.refresh();
        outcome
    }

    pub fn confirm_start(&self) {
        self.archive.update_untracked(CaptureArchive::confirm_start);
        self.refresh();
    }

    /// The start failed: return to `Idle` and keep the reason for the panel.
    pub fn fail_start(&self, reason: &str) {
        let at_ms = now();
        self.archive
            .update_untracked(|archive| archive.fail_start(at_ms));
        self.error.set(Some(reason.to_owned()));
        self.refresh();
    }

    /// Report an invalid command without changing an existing capture.
    pub fn report_error(&self, reason: &str) {
        self.error.set(Some(reason.to_owned()));
    }

    pub fn pause(&self) {
        let at_ms = now();
        self.archive
            .update_untracked(|archive| archive.pause(at_ms));
        self.refresh();
    }

    pub fn resume(&self, playback_active: bool) -> Result<(), StartRefusal> {
        let at_ms = now();
        let outcome = self
            .archive
            .update_untracked(|archive| archive.resume(playback_active, at_ms));
        if let Err(refusal) = outcome {
            self.error.set(Some(refusal.label().to_owned()));
        } else {
            self.error.set(None);
        }
        self.refresh();
        outcome
    }

    pub fn finish(&self, reason: StopReason) {
        let at_ms = now();
        self.archive
            .update_untracked(|archive| archive.finish(reason, at_ms));
        self.refresh();
    }

    pub fn select(&self, id: CaptureId) {
        self.archive.update_untracked(|archive| archive.select(id));
        self.refresh();
    }

    pub fn append_attempt(&self, record: AttemptRecord) -> AppendOutcome {
        let at_ms = now();
        let outcome = self
            .archive
            .update_untracked(|archive| archive.append_attempt(record, at_ms));
        if outcome != AppendOutcome::Duplicate {
            self.refresh();
        }
        outcome
    }

    /// Retain one host emit. Summarized here so the observer closure stays
    /// trivial; `sheet_name` resolves through the model roster at capture
    /// time.
    pub fn note_batch(
        &self,
        batch_id: HostBatchId,
        events: &[SpreadsheetEvent],
        sheet_name: impl Fn(u32) -> Option<String>,
    ) {
        let at_ms = now();
        self.archive.update_untracked(|archive| {
            archive.note_batch(batch_id, events, at_ms, sheet_name);
        });
        self.refresh();
    }

    pub fn note_mutation(&self, sample: MutationSample) {
        let at_ms = now();
        self.archive
            .update_untracked(|archive| archive.note_mutation(sample, at_ms));
        self.refresh();
    }

    pub fn claim_pending_batches(&self) -> Vec<HostBatchId> {
        self.archive
            .update_untracked(CaptureArchive::claim_pending_batches)
    }

    pub fn retry_link(&self) -> Option<AttemptKey> {
        self.archive.with_untracked(CaptureArchive::retry_link)
    }

    pub fn remember_held(&self, key: AttemptKey, held: bool) {
        self.archive
            .update_untracked(|archive| archive.remember_held(key, held));
    }

    /// Note that a paint recording started, or was already running when this
    /// capture started.
    pub fn note_paint_recording(&self) {
        self.archive
            .update_untracked(CaptureArchive::note_paint_recording);
        self.refresh();
    }

    pub fn delete_capture(&self, id: CaptureId) -> bool {
        let deleted = self
            .archive
            .update_untracked(|archive| archive.delete_capture(id));
        if deleted {
            self.refresh();
        }
        deleted
    }

    /// Close the active capture and open a new generation.
    pub fn end_generation(&self) {
        let at_ms = now();
        self.archive
            .update_untracked(|archive| archive.end_generation(at_ms));
        self.refresh();
    }

    /// Ask the capture Effect to re-derive the canvas flag. Called by paths
    /// that stop a capture without a command: a limit stop from the render
    /// loop, or a generation change.
    pub fn request_canvas_sync(&self) {
        self.sync_request
            .update(|request| *request = request.wrapping_add(1));
    }

    pub fn canvas_sync_requested(&self) -> u64 {
        self.sync_request.get()
    }

    /// Publish the archive-derived facts. Bumps the canvas sync request only
    /// when the capture state changed, so a plain append costs the capture
    /// Effect nothing.
    fn refresh(&self) {
        let (state, status) = self
            .archive
            .with_untracked(|archive| (archive.state(), archive.status()));
        let state_changed = self.state.get_untracked() != state;
        if state_changed {
            self.state.set(state);
        }
        self.status.set(status);
        self.revision
            .update(|revision| *revision = revision.wrapping_add(1));
        if state_changed {
            self.request_canvas_sync();
        }
    }
}
