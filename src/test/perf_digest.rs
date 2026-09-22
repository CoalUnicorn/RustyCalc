//! Digest rules: separate categories, measured samples, and pure timing.
//!
//! The digest is a pure function of the capture, the filter, and the
//! observation timestamp, so these tests build the records directly and pin
//! every timestamp. No browser, no canvas, no signals.

use crate::perf::{
    AttemptKey, AttemptOrigin, AttemptRecord, CaptureRecord, DigestFilter, EvaluationOutcome,
    InstrumentationFlags, MutationOutcome, MutationSample, StopReason, digest,
};
use iron_canvas_core::{
    DIAG_SCHEMA_VERSION, DiagCacheResolution, DiagFetch, DiagPaintCounts, DiagPaintedLayers,
    DiagRepaint, FrameDiagnostics, FrameOutcome, GridVerdict, RenderStrategy,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn diagnostics(attempt_seq: u64) -> FrameDiagnostics {
    FrameDiagnostics {
        schema_version: DIAG_SCHEMA_VERSION,
        attempt_seq,
        ..FrameDiagnostics::default()
    }
}

fn attempt(attempt_seq: u64, diagnostics: FrameDiagnostics) -> AttemptRecord {
    AttemptRecord {
        key: AttemptKey {
            generation: 0,
            attempt_seq,
        },
        captured_at_ms: 100.0,
        render_call_ms: Some(4.0),
        origin: AttemptOrigin::Live,
        retry_of: None,
        batch_ids: Vec::new(),
        sheet_name: Some("Sheet1".to_owned()),
        backing_size: (1000, 750),
        diagnostics,
    }
}

fn capture(attempts: Vec<AttemptRecord>, mutations: Vec<MutationSample>) -> CaptureRecord {
    CaptureRecord {
        id: 1,
        name: "capture 1".to_owned(),
        generation: 0,
        started_at_ms: 100.0,
        completed_at_ms: Some(200.0),
        paused: Vec::new(),
        pause_started_at_ms: None,
        stop_reason: Some(StopReason::Finished),
        instrumentation: InstrumentationFlags::default(),
        sheet_names: Vec::new(),
        attempts,
        batches: Vec::new(),
        mutations,
        truncated: false,
        rejected_records: 0,
    }
}

#[wasm_bindgen_test]
fn overlapping_paint_categories_stay_separate() {
    let mut grid = diagnostics(1);
    grid.painted_layers = DiagPaintedLayers {
        grid: true,
        overlay: false,
    };

    let mut overlay = diagnostics(2);
    overlay.painted_layers = DiagPaintedLayers {
        grid: false,
        overlay: true,
    };

    // Both layers painted: a grid paint, and never an overlay-only paint.
    let mut both = diagnostics(3);
    both.painted_layers = DiagPaintedLayers {
        grid: true,
        overlay: true,
    };

    // A skip paints no layer.
    let mut skip = diagnostics(4);
    skip.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Skip),
        ..DiagRepaint::default()
    };

    let record = capture(
        vec![
            attempt(1, grid),
            attempt(2, overlay),
            attempt(3, both),
            attempt(4, skip),
        ],
        Vec::new(),
    );
    let out = digest(&record, &DigestFilter::default(), 200.0);

    assert_eq!(out.attempts, 4);
    assert_eq!(out.grid_paints, 2);
    assert_eq!(out.overlay_only_paints, 1);
    assert_eq!(out.skips, 1);
}

#[wasm_bindgen_test]
fn render_statistics_exclude_unmeasured_attempts() {
    let mut untimed = attempt(2, diagnostics(2));
    untimed.render_call_ms = None;

    let record = capture(vec![attempt(1, diagnostics(1)), untimed], Vec::new());
    let out = digest(&record, &DigestFilter::default(), 200.0);

    assert_eq!(out.attempts, 2);
    assert_eq!(out.render.measured, 1);
    assert_eq!(out.render.total_ms, 4.0);
    assert_eq!(out.render.median_ms, Some(4.0));
    assert_eq!(out.render.max_ms, Some(4.0));
    assert_eq!(out.render_time_ms, 4.0);
}

#[wasm_bindgen_test]
fn render_statistics_are_none_when_nothing_was_measured() {
    let mut untimed = attempt(1, diagnostics(1));
    untimed.render_call_ms = None;

    let record = capture(vec![untimed], Vec::new());
    let out = digest(&record, &DigestFilter::default(), 200.0);

    assert_eq!(out.render.measured, 0);
    assert_eq!(out.render.total_ms, 0.0);
    assert_eq!(out.render.median_ms, None);
    assert_eq!(out.render.max_ms, None);
    assert_eq!(out.render_time_ms, 0.0);
}

#[wasm_bindgen_test]
fn active_time_excludes_closed_and_open_pauses() {
    let mut record = capture(vec![attempt(1, diagnostics(1))], Vec::new());
    record.completed_at_ms = None;
    record.paused = vec![(150.0, 170.0)];
    record.pause_started_at_ms = Some(180.0);

    let out = digest(&record, &DigestFilter::default(), 200.0);

    // wall: 200 - 100. active: 100 - 20 closed - 20 open.
    assert_eq!(out.wall_ms, 100.0);
    assert_eq!(out.active_ms, 60.0);
}

#[wasm_bindgen_test]
fn forced_baseline_is_excluded_by_default_and_counted() {
    let mut forced = attempt(2, diagnostics(2));
    forced.origin = AttemptOrigin::ForcedBaseline;
    let record = capture(vec![attempt(1, diagnostics(1)), forced], Vec::new());

    let default = digest(&record, &DigestFilter::default(), 200.0);
    assert_eq!(default.attempts, 1);
    assert_eq!(default.forced_excluded, 1);

    let included = digest(
        &record,
        &DigestFilter {
            include_forced_baseline: true,
            ..DigestFilter::default()
        },
        200.0,
    );
    assert_eq!(included.attempts, 2);
    assert_eq!(included.forced_excluded, 0);
}

#[wasm_bindgen_test]
fn mutation_sample_counts_once_across_attempts() {
    let mutation = MutationSample {
        seq: 1,
        started_at_ms: 110.0,
        apply_ms: 3.0,
        outcome: MutationOutcome::Err,
        evaluation: EvaluationOutcome::NotRun,
    };
    let record = capture(
        vec![attempt(1, diagnostics(1)), attempt(2, diagnostics(2))],
        vec![mutation],
    );

    let out = digest(&record, &DigestFilter::default(), 200.0);

    assert_eq!(out.mutation_samples, 1);
    assert_eq!(out.mutation_err, 1);
    assert_eq!(out.mutation_ok, 0);
    assert_eq!(out.evaluation_not_run, 1);
    assert_eq!(out.evaluation_deferred, 0);
    assert_eq!(out.evaluation_measured, 0);
}

#[wasm_bindgen_test]
fn committed_held_and_fallback_are_counted_separately() {
    let mut committed = diagnostics(1);
    committed.committed_seq = Some(1);
    committed.selected = Some(RenderStrategy::ChangedCells);
    committed.effective = Some(RenderStrategy::ChangedCells);

    let mut held = diagnostics(2);
    held.cache.resolution = DiagCacheResolution::HeldForRetry;

    let mut fallback = diagnostics(3);
    fallback.selected = Some(RenderStrategy::ScrollBlit);
    fallback.effective = Some(RenderStrategy::FullRebuild);

    let mut fetched = diagnostics(4);
    fetched.fetch = DiagFetch {
        batches: 2,
        addressed_cells: 30,
        logical_slots: 12,
        requests: Vec::new(),
    };
    fetched.paint_counts = DiagPaintCounts { rows: 3, cells: 40 };

    let record = capture(
        vec![
            attempt(1, committed),
            attempt(2, held),
            attempt(3, fallback),
            attempt(4, fetched),
        ],
        Vec::new(),
    );
    let out = digest(&record, &DigestFilter::default(), 200.0);

    assert_eq!(out.attempts, 4);
    assert_eq!(out.committed, 1);
    assert_eq!(out.held, 1);
    assert_eq!(out.fallbacks, 1);
    assert_eq!(out.fetch.batches, 2);
    assert_eq!(out.fetch.addressed_cells, 30);
    assert_eq!(out.fetch.logical_slots, 12);
    assert_eq!(out.painted_cell_visits, 40);
}

#[wasm_bindgen_test]
fn filter_narrows_by_effective_strategy_and_outcome() {
    let mut changed = diagnostics(1);
    changed.effective = Some(RenderStrategy::ChangedCells);
    changed.outcome = FrameOutcome::Painted;

    let mut held = diagnostics(2);
    held.effective = Some(RenderStrategy::ScrollBlit);
    held.outcome = FrameOutcome::HeldOnBridgeFailure;
    held.cache.resolution = DiagCacheResolution::HeldForRetry;

    let record = capture(vec![attempt(1, changed), attempt(2, held)], Vec::new());

    let by_strategy = digest(
        &record,
        &DigestFilter {
            effective: Some(RenderStrategy::ChangedCells),
            ..DigestFilter::default()
        },
        200.0,
    );
    assert_eq!(by_strategy.attempts, 1);

    let by_outcome = digest(
        &record,
        &DigestFilter {
            outcome: Some(FrameOutcome::HeldOnBridgeFailure),
            ..DigestFilter::default()
        },
        200.0,
    );
    assert_eq!(by_outcome.attempts, 1);
    assert_eq!(by_outcome.held, 1);
}
