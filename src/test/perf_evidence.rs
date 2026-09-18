//! Address-evidence rules: labelled sources, precisions, and coverage.
//!
//! Every fixture is a `FrameDiagnostics` literal. No renderer runs, so each
//! case pins exactly the executed facts the adapter decides from.

use crate::coord::SheetRange;
use crate::perf::{
    AddressEvidence, AttemptKey, AttemptOrigin, AttemptRecord, CaptureRecord, Coverage,
    EvidenceNote, EvidencePrecision, EvidenceSource, HostBatchKind, HostBatchSummary, HostScope,
    InstrumentationFlags, SheetRef, StopReason, UnavailableReason, attempt_evidence, format_range,
    repainted_coverage,
};
use iron_canvas_core::chrome::{GridShape, PaneRegion};
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::renderer::diag::{
    DiagBlit, DiagBlitResultTag, DiagCache, DiagCacheResolution, DiagChangedCell, DiagFetch,
    DiagFetchPurpose, DiagFetchRequest, DiagGeometry, DiagPaintedLayers, DiagRepaint,
    DiagRepaintReason, DiagRevealedStrip, DiagSegment, DiagSourceRange, FrameDiagnostics,
};
use iron_canvas_core::{
    CanvasSize, GridVerdict, PixelRect, Point, RCRange, RenderStrategy, RowSpan, WorkFlags,
};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const SHEET: u32 = 1;

fn segment(region: PaneRegion, r1: i32, c1: i32, r2: i32, c2: i32) -> DiagSegment {
    DiagSegment {
        region,
        range: RCRange { r1, c1, r2, c2 },
        cells: 0,
    }
}

fn geometry(segments: Vec<DiagSegment>) -> DiagGeometry {
    DiagGeometry {
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
        shape: GridShape::from_lens([0, 40], [0, 19]),
        segments,
    }
}

fn diagnostics() -> FrameDiagnostics {
    FrameDiagnostics {
        schema_version: 3,
        attempt_seq: 1,
        painted_layers: DiagPaintedLayers {
            grid: true,
            overlay: false,
        },
        cache: DiagCache {
            resolution: DiagCacheResolution::Committed,
            ..DiagCache::default()
        },
        geometry: Some(geometry(vec![segment(
            PaneRegion::BottomRight,
            1,
            1,
            40,
            19,
        )])),
        ..FrameDiagnostics::default()
    }
}

fn attempt(diagnostics: FrameDiagnostics) -> AttemptRecord {
    AttemptRecord {
        key: AttemptKey {
            generation: 0,
            attempt_seq: 1,
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

fn capture() -> CaptureRecord {
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
        attempts: Vec::new(),
        batches: Vec::new(),
        mutations: Vec::new(),
        truncated: false,
        rejected_records: 0,
    }
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> PixelRect {
    PixelRect {
        top_left: Point { x, y },
        width: w,
        height: h,
    }
}

fn revealed(region: PaneRegion, r1: i32, c1: i32, r2: i32, c2: i32) -> DiagRevealedStrip {
    DiagRevealedStrip {
        region,
        range: RCRange { r1, c1, r2, c2 },
    }
}

fn blit(result: DiagBlitResultTag, revealed: Vec<DiagRevealedStrip>) -> DiagBlit {
    DiagBlit {
        axis: Axis::Row,
        delta: 3,
        src: rect(0, 0, 100, 100),
        dst: rect(0, 30, 100, 100),
        clip: None,
        strip: rect(0, 100, 100, 30),
        revealed,
        result,
        cold_cache: None,
    }
}

fn row(rows: &[AddressEvidence], source: EvidenceSource) -> &AddressEvidence {
    rows.iter()
        .find(|row| row.source == source)
        .expect("row present")
}

/// The fingerprint-changes row alone, for the empty-list cases.
fn fingerprint_row(diagnostics: FrameDiagnostics) -> AddressEvidence {
    let rows = attempt_evidence(&capture(), &attempt(diagnostics));
    row(&rows, EvidenceSource::FingerprintChanges).clone()
}

#[wasm_bindgen_test]
fn changed_cells_source_range_and_clip_stay_distinct() {
    let mut d = diagnostics();
    d.effective = Some(RenderStrategy::ChangedCells);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Range),
        reason: Some(DiagRepaintReason::ChangedCells),
        changed_rows: Vec::new(),
        changed_cells: vec![
            DiagChangedCell { row: 4, column: 1 },
            DiagChangedCell { row: 4, column: 18 },
        ],
        clip: Some(rect(0, 60, 400, 20)),
        source_ranges: vec![DiagSourceRange {
            region: PaneRegion::BottomRight,
            range: RCRange {
                r1: 4,
                c1: 1,
                r2: 4,
                c2: 18,
            },
        }],
    };

    let rows = attempt_evidence(&capture(), &attempt(d));

    let changed = row(&rows, EvidenceSource::FingerprintChanges);
    assert_eq!(changed.precision, EvidencePrecision::Exact);
    assert_eq!(changed.ranges.len(), 2);
    assert_eq!(
        changed.ranges[0].range,
        RCRange {
            r1: 4,
            c1: 1,
            r2: 4,
            c2: 1
        }
    );
    assert_eq!(
        changed.ranges[1].range,
        RCRange {
            r1: 4,
            c1: 18,
            r2: 4,
            c2: 18
        }
    );

    let source = row(&rows, EvidenceSource::RepaintSourceRanges);
    assert_eq!(source.precision, EvidencePrecision::Exact);
    assert_eq!(source.ranges.len(), 1);
    assert_eq!(
        source.ranges[0].range,
        RCRange {
            r1: 4,
            c1: 1,
            r2: 4,
            c2: 18
        }
    );

    let clip = row(&rows, EvidenceSource::RepaintClip);
    assert_eq!(clip.precision, EvidencePrecision::Exact);
    assert_eq!(clip.clip, Some(rect(0, 60, 400, 20)));
    assert!(clip.ranges.is_empty());

    // The two cells were never merged into the enclosing source rectangle.
    assert_ne!(changed.ranges[0], source.ranges[0]);
    assert_ne!(changed.ranges[1], source.ranges[0]);
}

#[wasm_bindgen_test]
fn frozen_columns_keep_two_coverage_segments_for_one_row() {
    let mut d = diagnostics();
    d.geometry = Some(geometry(vec![
        segment(PaneRegion::TopLeft, 4, 1, 4, 2),
        segment(PaneRegion::BottomRight, 4, 3, 4, 19),
    ]));
    d.effective = Some(RenderStrategy::DamagedRows);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Rows { spans: 1, rows: 1 }),
        reason: Some(DiagRepaintReason::ChangedRows),
        changed_rows: vec![RowSpan::new(4, 4)],
        ..DiagRepaint::default()
    };

    match repainted_coverage(&d) {
        Coverage::Ranges(ranges) => {
            assert_eq!(ranges.len(), 2);
            assert_eq!(
                ranges[0].range,
                RCRange {
                    r1: 4,
                    c1: 1,
                    r2: 4,
                    c2: 2
                }
            );
            assert_eq!(
                ranges[1].range,
                RCRange {
                    r1: 4,
                    c1: 3,
                    r2: 4,
                    c2: 19
                }
            );
        }
        other => panic!("expected row coverage, got {other:?}"),
    }
}

#[wasm_bindgen_test]
fn held_blit_preflight_reports_no_coverage_and_a_held_note() {
    let mut d = diagnostics();
    d.cache.resolution = DiagCacheResolution::HeldForRetry;
    d.effective = Some(RenderStrategy::ScrollBlit);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Strip),
        ..DiagRepaint::default()
    };
    d.blit = Some(blit(
        DiagBlitResultTag::HeldPreflight,
        vec![revealed(PaneRegion::BottomRight, 20, 1, 30, 19)],
    ));

    assert_eq!(
        repainted_coverage(&d),
        Coverage::None {
            reason: UnavailableReason::HeldAttempt
        }
    );

    let rows = attempt_evidence(&capture(), &attempt(d));
    let coverage = row(&rows, EvidenceSource::RepaintedCoverage);
    assert_eq!(
        coverage.precision,
        EvidencePrecision::Unavailable(UnavailableReason::HeldAttempt)
    );
    assert_eq!(coverage.note, Some(EvidenceNote::HeldExcludesCoverage));
    assert!(coverage.ranges.is_empty());

    let revealed = row(&rows, EvidenceSource::RevealedRanges);
    assert_eq!(revealed.precision, EvidencePrecision::Exact);
    assert_eq!(revealed.ranges.len(), 1);
}

#[wasm_bindgen_test]
fn blit_fallback_reports_full_coverage_not_the_damage_strip() {
    let mut d = diagnostics();
    d.geometry = Some(geometry(vec![
        segment(PaneRegion::TopLeft, 1, 1, 40, 2),
        segment(PaneRegion::BottomRight, 1, 3, 40, 19),
    ]));
    d.effective = Some(RenderStrategy::ScrollBlit);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Full),
        ..DiagRepaint::default()
    };
    d.fetch = DiagFetch {
        batches: 1,
        addressed_cells: 10,
        logical_slots: 5,
        requests: vec![DiagFetchRequest {
            purpose: DiagFetchPurpose::DamageStrip,
            region: Some(PaneRegion::BottomRight),
            range: RCRange {
                r1: 20,
                c1: 1,
                r2: 30,
                c2: 19,
            },
            cells: 10,
            slots: 5,
        }],
    };
    d.blit = Some(blit(DiagBlitResultTag::GridFallback, Vec::new()));

    match repainted_coverage(&d) {
        Coverage::Ranges(ranges) => {
            assert_eq!(ranges.len(), 2);
            assert_eq!(
                ranges[1].range,
                RCRange {
                    r1: 1,
                    c1: 3,
                    r2: 40,
                    c2: 19
                }
            );
            assert!(
                ranges.iter().all(|range| range.range.r2 == 40),
                "the damage strip must not leak into full coverage"
            );
        }
        other => panic!("expected full coverage, got {other:?}"),
    }
}

#[wasm_bindgen_test]
fn view_only_blit_reports_revealed_coverage() {
    let mut d = diagnostics();
    d.work = WorkFlags::VIEW | WorkFlags::OVERLAY;
    d.effective = Some(RenderStrategy::ScrollBlit);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Strip),
        ..DiagRepaint::default()
    };
    d.blit = Some(blit(
        DiagBlitResultTag::Shifted,
        vec![revealed(PaneRegion::BottomRight, 20, 1, 30, 19)],
    ));

    match repainted_coverage(&d) {
        Coverage::Ranges(ranges) => {
            assert_eq!(ranges.len(), 1);
            assert_eq!(
                ranges[0].range,
                RCRange {
                    r1: 20,
                    c1: 1,
                    r2: 30,
                    c2: 19
                }
            );
        }
        other => panic!("expected revealed coverage, got {other:?}"),
    }
}

#[wasm_bindgen_test]
fn view_only_full_fallback_reports_full_coverage() {
    let mut d = diagnostics();
    d.work = WorkFlags::VIEW | WorkFlags::OVERLAY;
    d.effective = Some(RenderStrategy::ScrollBlit);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Full),
        ..DiagRepaint::default()
    };
    d.blit = Some(blit(DiagBlitResultTag::GridFallback, Vec::new()));

    match repainted_coverage(&d) {
        Coverage::Ranges(ranges) => assert_eq!(ranges.len(), 1),
        other => panic!("expected full coverage, got {other:?}"),
    }
}

#[wasm_bindgen_test]
fn an_empty_fingerprint_list_is_never_a_zero_change_list() {
    // A full rebuild never runs the comparison.
    let mut full_rebuild = diagnostics();
    full_rebuild.selected = Some(RenderStrategy::FullRebuild);
    full_rebuild.effective = Some(RenderStrategy::FullRebuild);
    let unreported = fingerprint_row(full_rebuild);
    assert_eq!(
        unreported.precision,
        EvidencePrecision::Unavailable(UnavailableReason::NotCompared)
    );
    assert!(unreported.ranges.is_empty());
    assert_eq!(unreported.note, None);

    // Missing painted history.
    let mut no_history = diagnostics();
    no_history.effective = Some(RenderStrategy::ChangedCells);
    no_history.repaint = DiagRepaint {
        reason: Some(DiagRepaintReason::NoPaintedHistory),
        ..DiagRepaint::default()
    };
    assert_eq!(
        fingerprint_row(no_history).precision,
        EvidencePrecision::Unavailable(UnavailableReason::NoPaintedHistory)
    );

    // A committed/candidate layout mismatch.
    let mut mismatch = diagnostics();
    mismatch.effective = Some(RenderStrategy::ChangedCells);
    mismatch.repaint = DiagRepaint {
        reason: Some(DiagRepaintReason::LayoutMismatch),
        ..DiagRepaint::default()
    };
    assert_eq!(
        fingerprint_row(mismatch).precision,
        EvidencePrecision::Unavailable(UnavailableReason::LayoutMismatch)
    );

    // A row-address divergence produced no change list either.
    let mut divergence = diagnostics();
    divergence.effective = Some(RenderStrategy::DamagedRows);
    divergence.repaint = DiagRepaint {
        reason: Some(DiagRepaintReason::RowAddressMismatch),
        ..DiagRepaint::default()
    };
    assert_eq!(
        fingerprint_row(divergence).precision,
        EvidencePrecision::Unavailable(UnavailableReason::NotCompared)
    );

    // An overlay-only attempt never entered the grid.
    let mut overlay = diagnostics();
    overlay.painted_layers = DiagPaintedLayers {
        grid: false,
        overlay: true,
    };
    overlay.effective = Some(RenderStrategy::OverlayOnly);
    let overlay_row = fingerprint_row(overlay.clone());
    assert_eq!(
        overlay_row.precision,
        EvidencePrecision::Unavailable(UnavailableReason::NoGridWork)
    );
    assert_eq!(
        repainted_coverage(&overlay),
        Coverage::None {
            reason: UnavailableReason::NoGridWork
        }
    );

    // A skip after an equal comparison is an exact statement of no change.
    let mut skip = diagnostics();
    skip.effective = Some(RenderStrategy::ChangedCells);
    skip.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Skip),
        reason: Some(DiagRepaintReason::FingerprintsEqual),
        ..DiagRepaint::default()
    };
    let equal = fingerprint_row(skip.clone());
    assert_eq!(equal.precision, EvidencePrecision::Exact);
    assert!(equal.ranges.is_empty());
    assert_eq!(equal.note, Some(EvidenceNote::NoVisibleFingerprintChanges));
    assert_eq!(
        repainted_coverage(&skip),
        Coverage::None {
            reason: UnavailableReason::NoGridWork
        }
    );
}

#[wasm_bindgen_test]
fn a_batch_id_resolves_to_its_own_summaries_only() {
    let mut record = capture();
    record.batches = vec![
        summary(1, 4, 1, 4, 4),
        summary(2, 7, 1, 7, 9),
        summary(3, 9, 1, 9, 9),
    ];

    let mut tied = attempt(diagnostics());
    tied.batch_ids = vec![2];
    let rows = attempt_evidence(&record, &tied);
    let reported = row(&rows, EvidenceSource::ReportedChanges);

    assert_eq!(reported.precision, EvidencePrecision::Exact);
    assert_eq!(reported.ranges.len(), 1);
    assert_eq!(
        reported.ranges[0].range,
        RCRange {
            r1: 7,
            c1: 1,
            r2: 7,
            c2: 9
        }
    );
}

fn summary(batch_id: u64, r1: i32, c1: i32, r2: i32, c2: i32) -> HostBatchSummary {
    HostBatchSummary {
        batch_id,
        index: 0,
        at_ms: 90.0,
        kind: HostBatchKind::Content,
        scope: Some(HostScope::Range(SheetRange::new(SHEET, r1, c1, r2, c2))),
        sheet: Some(SheetRef::new(SHEET, Some("Data".to_owned()))),
    }
}

#[wasm_bindgen_test]
fn host_batches_resolve_and_a_dangling_id_is_unavailable() {
    let mut record = capture();
    record.batches = vec![HostBatchSummary {
        batch_id: 7,
        index: 0,
        at_ms: 90.0,
        kind: HostBatchKind::Content,
        scope: Some(HostScope::Range(SheetRange::new(SHEET, 4, 1, 4, 18))),
        sheet: Some(SheetRef::new(SHEET, Some("Data".to_owned()))),
    }];

    let mut d = diagnostics();
    d.effective = Some(RenderStrategy::ChangedCells);
    d.fetch = DiagFetch {
        batches: 1,
        addressed_cells: 5,
        logical_slots: 5,
        requests: vec![DiagFetchRequest {
            purpose: DiagFetchPurpose::FullSegment,
            region: Some(PaneRegion::BottomRight),
            range: RCRange {
                r1: 4,
                c1: 3,
                r2: 4,
                c2: 7,
            },
            cells: 5,
            slots: 5,
        }],
    };
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Range),
        reason: Some(DiagRepaintReason::ChangedCells),
        changed_cells: vec![DiagChangedCell { row: 4, column: 5 }],
        source_ranges: vec![DiagSourceRange {
            region: PaneRegion::BottomRight,
            range: RCRange {
                r1: 4,
                c1: 3,
                r2: 4,
                c2: 7,
            },
        }],
        clip: None,
        changed_rows: Vec::new(),
    };

    let mut tied = attempt(d);
    tied.batch_ids = vec![7];
    let rows = attempt_evidence(&record, &tied);

    let reported = row(&rows, EvidenceSource::ReportedChanges);
    assert_eq!(reported.precision, EvidencePrecision::Exact);
    assert_eq!(reported.ranges.len(), 1);
    assert_eq!(reported.ranges[0].sheet.name.as_deref(), Some("Data"));
    assert_eq!(
        reported.ranges[0].range,
        RCRange {
            r1: 4,
            c1: 1,
            r2: 4,
            c2: 18
        }
    );

    let fetched = row(&rows, EvidenceSource::FetchRequests);
    assert_eq!(fetched.ranges.len(), 1);
    assert_eq!(
        fetched.ranges[0].range,
        RCRange {
            r1: 4,
            c1: 3,
            r2: 4,
            c2: 7
        }
    );
    assert_ne!(reported.ranges[0], fetched.ranges[0]);
    assert_ne!(
        reported.ranges[0],
        row(&rows, EvidenceSource::RepaintSourceRanges).ranges[0]
    );

    let mut dangling_attempt = attempt(diagnostics());
    dangling_attempt.batch_ids = vec![99];
    let dangling = attempt_evidence(&record, &dangling_attempt);
    let missing = row(&dangling, EvidenceSource::ReportedChanges);
    assert_eq!(
        missing.precision,
        EvidencePrecision::Unavailable(UnavailableReason::SectionMissing)
    );
    assert!(missing.ranges.is_empty());
}

#[wasm_bindgen_test]
fn historical_addresses_use_the_captured_sheet_identity() {
    let mut record = capture();
    // A live roster that disagrees with history must never resolve an address.
    record.sheet_names = vec![(SHEET, "Renamed".to_owned())];
    record.batches = vec![HostBatchSummary {
        batch_id: 3,
        index: 0,
        at_ms: 90.0,
        kind: HostBatchKind::Structure,
        scope: Some(HostScope::Range(SheetRange::new(9, 4, 1, 4, 18))),
        sheet: Some(SheetRef::new(9, Some("Deleted".to_owned()))),
    }];

    let mut d = diagnostics();
    d.effective = Some(RenderStrategy::FullRebuild);
    d.repaint = DiagRepaint {
        verdict: Some(GridVerdict::Full),
        ..DiagRepaint::default()
    };
    let mut tied = attempt(d);
    tied.sheet_name = Some("Old Name".to_owned());
    tied.batch_ids = vec![3];

    let rows = attempt_evidence(&record, &tied);

    let coverage = row(&rows, EvidenceSource::RepaintedCoverage);
    assert_eq!(
        coverage.ranges[0].sheet,
        SheetRef::new(SHEET, Some("Old Name".to_owned()))
    );
    assert_eq!(
        format_range(&coverage.ranges[0].sheet, coverage.ranges[0].range),
        "Old Name!A1:S40"
    );

    let reported = row(&rows, EvidenceSource::ReportedChanges);
    assert_eq!(
        reported.ranges[0].sheet,
        SheetRef::new(9, Some("Deleted".to_owned()))
    );
    assert_eq!(
        format_range(&reported.ranges[0].sheet, reported.ranges[0].range),
        "Deleted!A4:R4"
    );
}

#[wasm_bindgen_test]
fn format_range_renders_a1_with_and_without_a_name() {
    let named = SheetRef::new(2, Some("Sheet1".to_owned()));
    assert_eq!(
        format_range(
            &named,
            RCRange {
                r1: 4,
                c1: 1,
                r2: 4,
                c2: 18
            }
        ),
        "Sheet1!A4:R4"
    );
    assert_eq!(
        format_range(
            &named,
            RCRange {
                r1: 4,
                c1: 1,
                r2: 4,
                c2: 1
            }
        ),
        "Sheet1!A4"
    );

    let unnamed = SheetRef::new(2, None);
    assert_eq!(
        format_range(
            &unnamed,
            RCRange {
                r1: 1,
                c1: 1,
                r2: 4,
                c2: 4
            }
        ),
        "Sheet#2!A1:D4"
    );
}
