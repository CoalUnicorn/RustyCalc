//! Capture archive rules: admission, provenance, and every limit.
//!
//! The archive is pure, so these tests drive it directly with fixed clocks —
//! no browser, no canvas, no signals.

use crate::Owner;
use crate::coord::{CellAddress, SheetRange};
use crate::events::{
    ContentEvent, FormatEvent, NavigationEvent, SpreadsheetEvent, StructureEvent, ThemeEvent,
};
use crate::model::CssColor;
use crate::perf::{
    AppendOutcome, AttemptKey, AttemptOrigin, AttemptRecord, CaptureArchive, CaptureState,
    HostBatchId, InstrumentationFlags, LimitKind, PerfStore, StartRefusal, StopReason, summarize,
};
use iron_canvas_core::chrome::{GridShape, PaneRegion};
use iron_canvas_core::renderer::diag::{
    DiagFetchPurpose, DiagFetchRequest, DiagGeometry, DiagSegment, FrameDiagnostics,
};
use iron_canvas_core::{CanvasSize, RCRange};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SHEET: u32 = 0;

/// A capture with one geometry fact, so byte estimation and the export have
/// something real to walk.
fn diagnostics(attempt_seq: u64) -> FrameDiagnostics {
    FrameDiagnostics {
        schema_version: 3,
        attempt_seq,
        geometry: Some(DiagGeometry {
            canvas: CanvasSize { w: 800.0, h: 600.0 },
            backing_size: (1000, 750),
            dpr: 1.25,
            sheet: SHEET,
            top_row: 1,
            left_column: 1,
            row_header_thickness: 40,
            col_header_thickness: 20,
            show_row_headers: true,
            show_col_headers: true,
            shape: GridShape::from_lens([1, 39], [1, 12]),
            segments: vec![DiagSegment {
                region: PaneRegion::BottomRight,
                range: RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 39,
                    c2: 12,
                },
                cells: 468,
            }],
        }),
        ..FrameDiagnostics::default()
    }
}

fn attempt(generation: u64, attempt_seq: u64) -> AttemptRecord {
    AttemptRecord {
        key: AttemptKey {
            generation,
            attempt_seq,
        },
        captured_at_ms: 100.0,
        render_call_ms: Some(4.0),
        origin: AttemptOrigin::Live,
        retry_of: None,
        batch_ids: Vec::new(),
        sheet_name: Some("Sheet1".to_owned()),
        backing_size: (1000, 750),
        diagnostics: diagnostics(attempt_seq),
    }
}

fn content(event: ContentEvent) -> SpreadsheetEvent {
    SpreadsheetEvent::Content(event)
}

fn format(event: FormatEvent) -> SpreadsheetEvent {
    SpreadsheetEvent::Format(event)
}

fn structure(event: StructureEvent) -> SpreadsheetEvent {
    SpreadsheetEvent::Structure(event)
}

fn navigation(event: NavigationEvent) -> SpreadsheetEvent {
    SpreadsheetEvent::Navigation(event)
}

fn theme(event: ThemeEvent) -> SpreadsheetEvent {
    SpreadsheetEvent::Theme(event)
}

fn archive_with_capture() -> CaptureArchive {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, vec![(SHEET, "Sheet1".to_owned())])
        .expect("start");
    archive.confirm_start();
    archive
}

fn content_batch(archive: &mut CaptureArchive, batch_id: HostBatchId, at_ms: f64) {
    let events = vec![crate::events::SpreadsheetEvent::Content(
        crate::events::ContentEvent::CellChanged {
            address: crate::coord::CellAddress {
                sheet: SHEET,
                row: 1,
                column: 1,
            },
            old_value: None,
            new_value: None,
        },
    )];
    archive.note_batch(batch_id, &events, at_ms, |_| Some("Sheet1".to_owned()));
}

#[wasm_bindgen_test]
fn duplicate_attempt_key_appends_once() {
    let mut archive = archive_with_capture();
    assert_eq!(
        archive.append_attempt(attempt(0, 7), 10.0),
        AppendOutcome::Appended
    );
    assert_eq!(
        archive.append_attempt(attempt(0, 7), 11.0),
        AppendOutcome::Duplicate
    );
    assert_eq!(
        archive.with_active(|capture| capture.attempts.len()),
        Some(1)
    );
}

#[wasm_bindgen_test]
fn held_attempt_and_retry_are_distinct_and_linked() {
    let mut archive = archive_with_capture();
    let held = attempt(0, 3);
    let held_key = held.key;
    assert_eq!(archive.append_attempt(held, 5.0), AppendOutcome::Appended);
    archive.remember_held(held_key, true);
    assert_eq!(archive.retry_link(), Some(held_key));

    let mut retry = attempt(0, 4);
    retry.retry_of = archive.retry_link();
    assert_eq!(archive.append_attempt(retry, 6.0), AppendOutcome::Appended);

    let (first, second) = archive
        .with_active(|capture| (capture.attempts[0].clone(), capture.attempts[1].clone()))
        .expect("active");
    assert_ne!(first.key, second.key);
    assert_eq!(second.retry_of, Some(first.key));
    assert_eq!(first.retry_of, None, "a hold retries nothing");
}

#[wasm_bindgen_test]
fn retry_links_survive_new_batches_and_consecutive_holds() {
    let mut archive = archive_with_capture();
    content_batch(&mut archive, 1, 1.0);

    // A successful attempt owns the first batch.
    let mut a = attempt(0, 1);
    a.batch_ids = archive.claim_pending_batches();
    assert_eq!(archive.append_attempt(a, 2.0), AppendOutcome::Appended);

    // A hold claims the next batch and becomes the held attempt.
    let mut b = attempt(0, 2);
    content_batch(&mut archive, 2, 3.0);
    b.batch_ids = archive.claim_pending_batches();
    let b_key = b.key;
    assert_eq!(archive.append_attempt(b, 4.0), AppendOutcome::Appended);
    archive.remember_held(b_key, true);

    // A retry names the hold and claims the batch that followed it.
    let mut c = attempt(0, 3);
    content_batch(&mut archive, 3, 5.0);
    c.retry_of = archive.retry_link();
    c.batch_ids = archive.claim_pending_batches();
    assert_eq!(archive.append_attempt(c, 6.0), AppendOutcome::Appended);

    // Two consecutive holds: each remembers itself in turn.
    let d = attempt(0, 4);
    let d_key = d.key;
    archive.remember_held(d_key, true);
    assert_eq!(archive.append_attempt(d, 7.0), AppendOutcome::Appended);
    assert_eq!(archive.retry_link(), Some(d_key));

    let attempts = archive
        .with_active(|capture| capture.attempts.clone())
        .expect("active");
    assert_eq!(attempts[0].batch_ids, vec![1]);
    assert_eq!(attempts[1].batch_ids, vec![2], "a hold claims its batch");
    assert_eq!(attempts[2].batch_ids, vec![3], "the retry claims its own");
    assert_eq!(attempts[2].retry_of, Some(b_key));
    assert_eq!(attempts[3].batch_ids, Vec::<HostBatchId>::new());

    // A new capture keeps no thread from the previous one.
    archive.finish(StopReason::Finished, 8.0);
    archive
        .request_start(false, 9.0, Vec::new())
        .expect("second capture");
    archive.confirm_start();
    assert_eq!(archive.retry_link(), None);
    assert_eq!(archive.claim_pending_batches(), Vec::<HostBatchId>::new());
}

#[wasm_bindgen_test]
fn forced_baseline_claims_nothing() {
    let mut archive = archive_with_capture();
    content_batch(&mut archive, 9, 1.0);

    let mut baseline = attempt(0, 1);
    baseline.origin = AttemptOrigin::ForcedBaseline;
    baseline.retry_of = None;
    assert_eq!(
        archive.append_attempt(baseline, 2.0),
        AppendOutcome::Appended
    );
    assert_eq!(archive.retry_link(), None);

    // The pending batch is still there for the first live attempt.
    let mut live = attempt(0, 2);
    live.batch_ids = archive.claim_pending_batches();
    assert_eq!(archive.append_attempt(live, 3.0), AppendOutcome::Appended);

    let capture = archive.with_active(Clone::clone).expect("active");
    assert_eq!(capture.attempts[0].batch_ids, Vec::<HostBatchId>::new());
    assert_eq!(capture.attempts[1].batch_ids, vec![9]);
    assert_eq!(capture.instrumentation.forced_baselines, 1);
    assert!(!capture.instrumentation.paint_recording);
}

#[wasm_bindgen_test]
fn idle_ticks_append_nothing() {
    let mut archive = CaptureArchive::new();
    assert_eq!(
        archive.append_attempt(attempt(0, 1), 1.0),
        AppendOutcome::Rejected(StopReason::Finished)
    );
    assert_eq!(archive.state(), CaptureState::Idle);
    assert_eq!(archive.retained_bytes(), 0);
}

#[wasm_bindgen_test]
fn attempt_limit_stops_the_capture_and_keeps_the_prefix() {
    let mut archive = archive_with_capture();
    for seq in 1..=crate::perf::MAX_ATTEMPTS as u64 {
        assert_eq!(
            archive.append_attempt(attempt(0, seq), 1.0),
            AppendOutcome::Appended
        );
    }
    assert_eq!(
        archive.append_attempt(attempt(0, 999), 1.0),
        AppendOutcome::Rejected(StopReason::LimitReached(LimitKind::Attempts))
    );

    let capture = archive
        .with_capture(1, Clone::clone)
        .expect("the capture is retained");
    assert_eq!(capture.attempts.len(), crate::perf::MAX_ATTEMPTS);
    assert!(capture.truncated);
    assert_eq!(
        capture.stop_reason,
        Some(StopReason::LimitReached(LimitKind::Attempts))
    );
    assert_eq!(capture.rejected_records, 1);
    assert_eq!(archive.state(), CaptureState::Idle);
}

#[wasm_bindgen_test]
fn batch_limit_stops_the_capture_without_a_partial_batch() {
    let mut archive = archive_with_capture();
    let events = vec![
        crate::events::SpreadsheetEvent::Navigation(
            crate::events::NavigationEvent::SelectionChanged {
                address: crate::coord::CellAddress {
                    sheet: SHEET,
                    row: 1,
                    column: 1,
                },
            },
        ),
        crate::events::SpreadsheetEvent::Navigation(
            crate::events::NavigationEvent::EditingStarted {
                address: crate::coord::CellAddress {
                    sheet: SHEET,
                    row: 1,
                    column: 1,
                },
            },
        ),
    ];
    for batch_id in 1..=(crate::perf::MAX_BATCHES as u64 / 2) {
        archive.note_batch(batch_id, &events, 1.0, |_| Some("Sheet1".to_owned()));
    }
    let capture = archive.with_active(Clone::clone).expect("active");
    assert_eq!(capture.batches.len(), crate::perf::MAX_BATCHES);

    // The next batch would cross the limit: nothing from it is retained, and
    // the capture stops rather than dropping records quietly.
    archive.note_batch(999, &events, 2.0, |_| Some("Sheet1".to_owned()));
    let capture = archive
        .with_capture(1, Clone::clone)
        .expect("the capture is retained");
    assert_eq!(capture.batches.len(), crate::perf::MAX_BATCHES);
    assert_eq!(
        capture.rejected_records, 2,
        "both events of the refused batch"
    );
    assert!(capture.truncated);
    assert_eq!(
        capture.stop_reason,
        Some(StopReason::LimitReached(LimitKind::Batches))
    );
}

#[wasm_bindgen_test]
fn mutation_limit_stops_the_capture() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, Vec::new())
        .expect("start");
    archive.confirm_start();

    let sample = crate::perf::MutationSample {
        seq: 1,
        started_at_ms: 0.0,
        apply_ms: 1.0,
        outcome: crate::perf::MutationOutcome::Ok,
        evaluation: crate::perf::EvaluationOutcome::Deferred,
    };
    for _ in 0..crate::perf::MAX_MUTATIONS {
        archive.note_mutation(sample, 1.0);
    }
    archive.note_mutation(sample, 2.0);

    let capture = archive.with_capture(1, Clone::clone).expect("retained");
    assert_eq!(capture.mutations.len(), crate::perf::MAX_MUTATIONS);
    assert_eq!(capture.rejected_records, 1);
    assert_eq!(
        capture.stop_reason,
        Some(StopReason::LimitReached(LimitKind::Mutations))
    );
    assert!(archive.retained_bytes() < crate::perf::MAX_BYTES);
}

#[wasm_bindgen_test]
fn byte_budget_stops_the_capture_and_keeps_the_prefix() {
    let mut archive = archive_with_capture();
    assert_eq!(
        archive.append_attempt(attempt(0, 1), 1.0),
        AppendOutcome::Appended
    );

    // One record whose estimate alone crosses the budget: the fetch request
    // vector is the largest per-element charge the estimator walks.
    let mut oversized = attempt(0, 2);
    let mut diagnostics = diagnostics(2);
    diagnostics.fetch.requests = (0..1_500_000)
        .map(|_| DiagFetchRequest {
            purpose: DiagFetchPurpose::FullSegment,
            region: None,
            range: RCRange {
                r1: 1,
                c1: 1,
                r2: 1,
                c2: 1,
            },
            cells: 1,
            slots: 1,
        })
        .collect();
    oversized.diagnostics = diagnostics;
    // Precondition of the scenario, not a restatement of the code: this record
    // has to be over budget on its own for the append to be the interesting
    // case.
    assert!(crate::perf::estimate_attempt_bytes(&oversized) > crate::perf::MAX_BYTES);

    assert_eq!(
        archive.append_attempt(oversized, 2.0),
        AppendOutcome::Rejected(StopReason::LimitReached(LimitKind::Bytes))
    );
    let capture = archive.with_capture(1, Clone::clone).expect("retained");
    assert_eq!(capture.attempts.len(), 1, "the prefix is kept");
    assert!(capture.truncated);
    assert_eq!(capture.rejected_records, 1);
}

#[wasm_bindgen_test]
fn deleting_a_capture_releases_its_bytes() {
    let mut archive = archive_with_capture();
    archive.append_attempt(attempt(0, 1), 1.0);
    let charged = archive.retained_bytes();
    assert!(charged > 0);

    archive.finish(StopReason::Finished, 2.0);
    assert_eq!(archive.retained_bytes(), charged, "finishing retains bytes");
    assert!(archive.delete_capture(1));
    assert_eq!(archive.retained_bytes(), 0);

    // The freed budget admits records again.
    archive
        .request_start(false, 3.0, Vec::new())
        .expect("start after delete");
    archive.confirm_start();
    assert_eq!(
        archive.append_attempt(attempt(1, 1), 4.0),
        AppendOutcome::Appended
    );
}

#[wasm_bindgen_test]
fn several_captures_share_one_budget() {
    let mut archive = CaptureArchive::new();
    for expected in 1..=3u64 {
        archive
            .request_start(false, 0.0, vec![(SHEET, "Sheet1".to_owned())])
            .expect("start");
        archive.confirm_start();
        archive.append_attempt(attempt(0, 1), 1.0);
        archive.finish(StopReason::Finished, 2.0);
        assert_eq!(archive.retained_captures(), expected as usize);
    }
    let one = archive.retained_bytes() / 3;
    let expected = one * 3;
    assert!(
        archive.retained_bytes() >= expected,
        "every capture is charged: {} >= {expected}",
        archive.retained_bytes()
    );
}

#[wasm_bindgen_test]
fn capture_limit_refuses_a_sixth_capture() {
    let mut archive = CaptureArchive::new();
    for _ in 0..crate::perf::MAX_CAPTURES {
        archive
            .request_start(false, 0.0, Vec::new())
            .expect("start");
        archive.confirm_start();
        archive.finish(StopReason::Finished, 1.0);
    }
    assert_eq!(
        archive.request_start(false, 2.0, Vec::new()),
        Err(StartRefusal::CaptureLimitReached)
    );
    let oldest = archive
        .with_selected(|capture| capture.id)
        .expect("selected");
    assert!(archive.delete_capture(oldest));
    assert!(archive.request_start(false, 3.0, Vec::new()).is_ok());
}

#[wasm_bindgen_test]
fn pause_and_resume_close_the_interval_without_dropping_records() {
    let mut archive = archive_with_capture();
    archive.append_attempt(attempt(0, 1), 1.0);
    archive.pause(10.0);
    assert_eq!(archive.state(), CaptureState::Paused(1));

    // Batches that arrive while paused are not claimed by a later attempt.
    content_batch(&mut archive, 5, 11.0);
    assert!(archive.resume(false, 20.0).is_ok());
    assert_eq!(archive.state(), CaptureState::Capturing(1));
    assert_eq!(archive.claim_pending_batches(), Vec::<HostBatchId>::new());

    archive.append_attempt(attempt(0, 2), 21.0);
    archive.finish(StopReason::Finished, 30.0);
    let capture = archive.with_capture(1, Clone::clone).expect("retained");
    assert_eq!(capture.attempts.len(), 2, "pause keeps every record");
    assert_eq!(capture.paused, vec![(10.0, 20.0)]);
    assert_eq!(capture.wall_ms(999.0), 30.0);
}

#[wasm_bindgen_test]
fn confirming_again_keeps_the_published_capture() {
    // The coordinator re-applies the canvas flag on resume, so `confirm_start`
    // runs on a capture that is already published. That must not restart or
    // drop it: the smoke test caught exactly this, with a resume emptying the
    // archive.
    let mut archive = archive_with_capture();
    archive.append_attempt(attempt(0, 1), 1.0);
    let charged = archive.retained_bytes();

    archive.confirm_start();
    assert_eq!(archive.state(), CaptureState::Capturing(1));
    assert_eq!(
        archive.with_active(|capture| capture.attempts.len()),
        Some(1)
    );
    assert_eq!(archive.retained_bytes(), charged);

    archive.pause(2.0);
    archive.confirm_start();
    assert_eq!(
        archive.state(),
        CaptureState::Paused(1),
        "a paused capture stays paused"
    );
    archive.finish(StopReason::Finished, 3.0);
    assert_eq!(
        archive.with_capture(1, |capture| capture.attempts.len()),
        Some(1)
    );
}

#[wasm_bindgen_test]
fn selecting_an_older_capture_never_retargets_the_active_slot() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, Vec::new())
        .expect("first");
    archive.confirm_start();
    archive.finish(StopReason::Finished, 1.0);

    archive
        .request_start(false, 2.0, Vec::new())
        .expect("second");
    archive.confirm_start();
    archive.select(1);

    assert_eq!(archive.selected(), Some(1));
    assert_eq!(archive.active_id(), Some(2));
    assert_eq!(
        archive.append_attempt(attempt(0, 1), 3.0),
        AppendOutcome::Appended
    );
    assert_eq!(
        archive.with_capture(2, |capture| capture.attempts.len()),
        Some(1)
    );
    assert_eq!(
        archive.with_capture(1, |capture| capture.attempts.len()),
        Some(0)
    );
    assert_eq!(archive.selected(), Some(1), "the selection is unchanged");
}

#[wasm_bindgen_test]
fn one_batch_per_emit_keeps_its_event_order() {
    let mut archive = archive_with_capture();
    let events = vec![
        crate::events::SpreadsheetEvent::Structure(crate::events::StructureEvent::WorksheetAdded {
            sheet: 1,
            name: "Second".to_owned(),
        }),
        crate::events::SpreadsheetEvent::Navigation(
            crate::events::NavigationEvent::ActiveSheetChanged {
                from_sheet: 0,
                to_sheet: 1,
            },
        ),
    ];
    archive.note_batch(4, &events, 1.0, |id| Some(format!("Sheet{id}")));
    let capture = archive.with_active(Clone::clone).expect("active");
    assert_eq!(capture.batches.len(), 2, "one summary per event");
    assert_eq!(capture.batches[0].batch_id, 4);
    assert_eq!(capture.batches[1].batch_id, 4);
    assert_eq!(capture.batches[0].index, 0);
    assert_eq!(capture.batches[1].index, 1);
    assert_eq!(
        capture.batches[1].sheet.as_ref().map(|sheet| sheet.id),
        Some(1)
    );
    assert_eq!(
        capture.batches[1]
            .sheet
            .as_ref()
            .and_then(|sheet| sheet.name.clone()),
        Some("Sheet1".to_owned())
    );
    assert_eq!(archive.claim_pending_batches(), vec![4]);
    assert_eq!(archive.claim_pending_batches(), Vec::<HostBatchId>::new());
}

#[wasm_bindgen_test]
fn generation_end_archives_the_capture_and_clears_the_memory() {
    let mut archive = archive_with_capture();
    archive.append_attempt(attempt(0, 1), 1.0);
    archive.remember_held(
        AttemptKey {
            generation: 0,
            attempt_seq: 1,
        },
        true,
    );
    content_batch(&mut archive, 3, 2.0);

    archive.end_generation(9.0);
    assert_eq!(archive.state(), CaptureState::Idle);
    assert_eq!(archive.generation(), 1);
    assert_eq!(archive.claim_pending_batches(), Vec::<HostBatchId>::new());
    assert_eq!(archive.retry_link(), None);

    let capture = archive.with_capture(1, Clone::clone).expect("archived");
    assert_eq!(capture.stop_reason, Some(StopReason::GenerationEnded));
    assert_eq!(capture.completed_at_ms, Some(9.0));
    assert_eq!(capture.attempts.len(), 1);
}

#[wasm_bindgen_test]
fn playback_refuses_start_and_resume() {
    let mut archive = CaptureArchive::new();
    assert_eq!(
        archive.request_start(true, 0.0, Vec::new()),
        Err(StartRefusal::PlaybackActive)
    );
    assert_eq!(
        archive.state(),
        CaptureState::Idle,
        "never reaches capturing"
    );

    archive
        .request_start(false, 1.0, Vec::new())
        .expect("start");
    archive.confirm_start();
    archive.pause(2.0);
    assert_eq!(archive.resume(true, 3.0), Err(StartRefusal::PlaybackActive));
    assert_eq!(archive.state(), CaptureState::Paused(1), "stays paused");

    // Playback entry closes the capture and keeps it.
    archive.finish(StopReason::PlaybackStarted, 4.0);
    let capture = archive.with_capture(1, Clone::clone).expect("retained");
    assert_eq!(capture.stop_reason, Some(StopReason::PlaybackStarted));
}

#[wasm_bindgen_test]
fn failing_start_releases_the_unpublished_capture() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, vec![(SHEET, "Sheet1".to_owned())])
        .expect("start");
    let charged = archive.retained_bytes();
    assert!(
        charged > 0,
        "the roster is charged before the canvas confirms"
    );
    assert_eq!(archive.state(), CaptureState::Starting);

    archive.fail_start(1.0);
    assert_eq!(archive.state(), CaptureState::Idle);
    assert_eq!(archive.retained_bytes(), 0);
    assert_eq!(archive.retained_captures(), 0);
    assert!(archive.request_start(false, 2.0, Vec::new()).is_ok());
}

#[wasm_bindgen_test]
fn summarize_covers_every_event_shape() {
    use crate::perf::{HostBatchKind, HostScope};

    let address = CellAddress {
        sheet: 2,
        row: 3,
        column: 4,
    };
    let range = SheetRange::new(2, 1, 1, 5, 5);
    let cases = vec![
        (
            content(ContentEvent::CellChanged {
                address,
                old_value: Some("1".to_owned()),
                new_value: Some("2".to_owned()),
            }),
            HostBatchKind::Content,
            Some(2),
        ),
        (
            content(ContentEvent::RangeChanged { sheet_area: range }),
            HostBatchKind::Content,
            Some(2),
        ),
        (
            content(ContentEvent::FormulaChanged { address }),
            HostBatchKind::Content,
            Some(2),
        ),
        (
            content(ContentEvent::CalculationUpdated {
                affected_sheets: vec![1, 2],
            }),
            HostBatchKind::Content,
            None,
        ),
        (
            content(ContentEvent::NamedRangesChanged),
            HostBatchKind::Content,
            None,
        ),
        (
            format(FormatEvent::CellStyleChanged { address }),
            HostBatchKind::Format,
            Some(2),
        ),
        (
            format(FormatEvent::RangeStyleChanged { area: range }),
            HostBatchKind::Format,
            Some(2),
        ),
        (
            format(FormatEvent::LayoutChanged {
                sheet: 2,
                col: Some(4),
                row: None,
            }),
            HostBatchKind::Format,
            Some(2),
        ),
        (
            format(FormatEvent::RecentColorsUpdated {
                colors: vec![CssColor::new("#fff"), CssColor::new("#000")],
            }),
            HostBatchKind::Format,
            None,
        ),
        (
            format(FormatEvent::DocumentColorsChanged {
                colors: vec![CssColor::new("#123456")],
            }),
            HostBatchKind::Format,
            None,
        ),
        (
            format(FormatEvent::ConditionalFormattingChanged { sheet: 2 }),
            HostBatchKind::Format,
            Some(2),
        ),
        (
            structure(StructureEvent::WorksheetAdded {
                sheet: 2,
                name: "Second".to_owned(),
            }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::WorksheetDeleted { sheet: 2 }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::WorksheetRenamed {
                sheet: 2,
                old_name: "a".to_owned(),
                new_name: "b".to_owned(),
            }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::WorksheetsReordered),
            HostBatchKind::Structure,
            None,
        ),
        (
            structure(StructureEvent::WorksheetHidden { sheet: 2 }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::WorksheetUnhidden {
                sheet: 2,
                name: "Second".to_owned(),
            }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::ColumnMoved {
                sheet: 2,
                from_col: 1,
                to_col: 3,
            }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::RowMoved {
                sheet: 2,
                from_row: 4,
                to_row: 9,
            }),
            HostBatchKind::Structure,
            Some(2),
        ),
        (
            structure(StructureEvent::DocumentReset),
            HostBatchKind::Structure,
            None,
        ),
        (
            navigation(NavigationEvent::SelectionChanged { address }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            navigation(NavigationEvent::SelectionRangeChanged { sheet_area: range }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            navigation(NavigationEvent::ViewportScrolled {
                sheet: 2,
                top_row: 10,
                left_col: 1,
            }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            navigation(NavigationEvent::ActiveSheetChanged {
                from_sheet: 0,
                to_sheet: 2,
            }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            navigation(NavigationEvent::EditingStarted { address }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            navigation(NavigationEvent::EditingEnded {
                address,
                committed: true,
            }),
            HostBatchKind::Navigation,
            Some(2),
        ),
        (
            theme(ThemeEvent::ThemeToggled {
                new_theme: crate::theme::Theme::Dark,
            }),
            HostBatchKind::Theme,
            None,
        ),
        (
            theme(ThemeEvent::PaletteUpdated),
            HostBatchKind::Theme,
            None,
        ),
        (
            theme(ThemeEvent::LocaleChanged {
                new_locale: "de".to_owned(),
            }),
            HostBatchKind::Theme,
            None,
        ),
    ];
    for (event, kind, sheet) in cases {
        let facts = summarize(&event);
        assert_eq!(facts.kind, kind);
        assert_eq!(facts.sheet, sheet, "sheet for {event:?}");
    }

    // Structures carry their header change, colors collapse to a count, and a
    // move keeps both endpoints. Nothing carries a value or a color list.
    let facts = summarize(&structure(StructureEvent::rows_inserted(
        crate::events::Location::new(2, 3, 4),
    )));
    assert!(matches!(facts.scope, Some(HostScope::Header(_))));
    let facts = summarize(&format(FormatEvent::RecentColorsUpdated {
        colors: vec![CssColor::new("#fff"), CssColor::new("#000")],
    }));
    assert_eq!(facts.scope, Some(HostScope::Colors { count: 2 }));
    let facts = summarize(&structure(StructureEvent::RowMoved {
        sheet: 2,
        from_row: 4,
        to_row: 9,
    }));
    assert_eq!(
        facts.scope,
        Some(HostScope::RowMoved {
            sheet: 2,
            from_row: 4,
            to_row: 9
        })
    );
}

#[wasm_bindgen_test]
fn store_publishes_state_and_revision_for_views() {
    let owner = Owner::new();
    owner.with(|| {
        let store = PerfStore::new();
        assert_eq!(store.status(), crate::perf::CaptureStatus::idle());

        store.request_start(false, Vec::new()).expect("start");
        assert_eq!(store.state(), CaptureState::Starting);
        let after_start = store.revision();
        store.confirm_start();
        assert_eq!(store.state(), CaptureState::Capturing(1));
        assert!(store.revision() > after_start);

        let record = attempt(1, 1);
        assert_eq!(store.append_attempt(record), AppendOutcome::Appended);
        assert_eq!(store.status().attempts, 1);
        assert_eq!(store.limit_report().retained_captures, 1);

        store.pause();
        assert_eq!(store.state(), CaptureState::Paused(1));
        store.finish(StopReason::Finished);
        assert_eq!(store.state(), CaptureState::Idle);
        assert_eq!(store.status().stop_reason, Some(StopReason::Finished));
    });
}

#[wasm_bindgen_test]
fn instrumentation_seeds_from_the_recording_that_overlaps() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, Vec::new())
        .expect("start");
    archive.confirm_start();
    archive.note_paint_recording();
    archive.note_paint_recording();
    archive.finish(StopReason::Finished, 1.0);
    let capture = archive.with_capture(1, Clone::clone).expect("retained");
    assert!(
        capture.instrumentation.paint_recording,
        "seeded once, never cleared"
    );
    assert_eq!(
        capture.instrumentation,
        InstrumentationFlags {
            paint_recording: true,
            forced_baselines: 0
        }
    );
}

#[wasm_bindgen_test]
fn capture_header_cannot_exceed_the_archive_budget() {
    let mut archive = CaptureArchive::new();
    assert!(
        archive
            .request_start(false, 0.0, vec![(0, "x".repeat(crate::perf::MAX_BYTES))])
            .is_err()
    );
    assert_eq!(archive.state(), CaptureState::Idle);
    assert_eq!(archive.retained_bytes(), 0);
}

#[wasm_bindgen_test]
fn pause_history_is_charged_and_released_on_delete() {
    let mut archive = archive_with_capture();
    let before = archive.retained_bytes();
    archive.pause(1.0);
    let paused = archive.retained_bytes();
    assert_eq!(paused, before + std::mem::size_of::<(f64, f64)>());
    archive.pause(2.0);
    assert_eq!(archive.retained_bytes(), paused);
    archive.resume(false, 3.0).unwrap();
    archive.pause(4.0);
    archive.finish(StopReason::Finished, 5.0);
    assert_eq!(
        archive.retained_bytes(),
        before + 2 * std::mem::size_of::<(f64, f64)>()
    );
    assert!(archive.delete_capture(1));
    assert_eq!(archive.retained_bytes(), 0);
}

#[wasm_bindgen_test]
fn paused_capture_rejects_attempts_and_retry_memory() {
    let mut archive = archive_with_capture();
    archive.pause(1.0);
    let before = archive.retained_bytes();
    let record = attempt(0, 1);
    archive.remember_held(record.key, true);
    assert!(matches!(
        archive.append_attempt(record, 2.0),
        AppendOutcome::Rejected(_)
    ));
    assert_eq!(archive.with_active(|c| c.attempts.len()), Some(0));
    assert_eq!(archive.retained_bytes(), before);
    assert_eq!(archive.retry_link(), None);
    assert_eq!(archive.state(), CaptureState::Paused(1));
}

#[wasm_bindgen_test]
fn event_bus_observer_sees_whole_batches_and_can_detach() {
    use leptos::prelude::*;
    use std::{cell::RefCell, rc::Rc};
    Owner::new().with(|| {
        let bus = crate::events::EventBus::new();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&seen);
        bus.set_batch_observer(Some(Rc::new(move |id, events| {
            sink.borrow_mut()
                .push((id, events.iter().map(summarize).collect::<Vec<_>>()));
            if id == 3 {
                bus.set_batch_observer(None);
            }
        })));
        bus.emit_events([
            content(ContentEvent::NamedRangesChanged),
            theme(ThemeEvent::PaletteUpdated),
        ]);
        assert_eq!(bus.content.get_untracked().len(), 1);
        assert_eq!(bus.theme.get_untracked().len(), 1);
        bus.emit_event(content(ContentEvent::NamedRangesChanged));
        bus.emit_event(theme(ThemeEvent::PaletteUpdated));
        bus.emit_event(content(ContentEvent::NamedRangesChanged));
        let seen = seen.borrow();
        assert_eq!(
            seen.iter()
                .map(|(id, events)| (*id, events.len()))
                .collect::<Vec<_>>(),
            vec![(1, 2), (2, 1), (3, 1)]
        );
        assert_eq!(seen[0].1[0].kind, crate::perf::HostBatchKind::Content);
        assert_eq!(seen[0].1[1].kind, crate::perf::HostBatchKind::Theme);
        assert_eq!(bus.content.get_untracked().len(), 1);
        assert!(bus.theme.get_untracked().is_empty());
    });
}

#[wasm_bindgen_test]
fn archive_budget_includes_headers_of_completed_captures() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(
            false,
            0.0,
            vec![(0, "x".repeat(crate::perf::MAX_BYTES / 2))],
        )
        .unwrap();
    archive.confirm_start();
    archive.finish(StopReason::Finished, 1.0);
    let retained = archive.retained_bytes();
    assert_eq!(
        archive.request_start(
            false,
            2.0,
            vec![(0, "y".repeat(crate::perf::MAX_BYTES / 2))]
        ),
        Err(StartRefusal::ByteBudgetExceeded)
    );
    assert_eq!(archive.retained_bytes(), retained);
    assert!(archive.delete_capture(1));
    assert!(archive.request_start(false, 3.0, Vec::new()).is_ok());
}

#[wasm_bindgen_test]
fn opening_a_pause_stops_at_the_byte_limit() {
    let mut archive = CaptureArchive::new();
    archive
        .request_start(false, 0.0, vec![(0, String::new())])
        .unwrap();
    let header = archive.retained_bytes();
    archive.fail_start(0.0);
    archive
        .request_start(
            false,
            0.0,
            vec![(0, "x".repeat(crate::perf::MAX_BYTES - header))],
        )
        .unwrap();
    archive.confirm_start();
    archive.pause(1.0);
    assert_eq!(archive.state(), CaptureState::Idle);
    assert_eq!(
        archive.status().stop_reason,
        Some(StopReason::LimitReached(LimitKind::Bytes))
    );
    assert_eq!(archive.retained_bytes(), crate::perf::MAX_BYTES);
}

#[wasm_bindgen_test]
fn deleted_sheet_does_not_take_the_next_sheets_name() {
    let mut archive = archive_with_capture();
    archive.note_batch(
        1,
        &[structure(StructureEvent::WorksheetDeleted { sheet: 0 })],
        1.0,
        |_| Some("Different sheet".into()),
    );
    let sheet = archive
        .with_active(|c| c.batches[0].sheet.clone())
        .flatten()
        .unwrap();
    assert_eq!(sheet.id, 0);
    assert_eq!(sheet.name, None);
}
