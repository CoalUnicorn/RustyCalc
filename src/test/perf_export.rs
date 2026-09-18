//! Export envelope rules: the stored snapshot is the only source, and the
//! embedded diagnostics are JSON objects.
//!
//! The fixtures are records only. Export must never read a canvas, so these
//! tests never build one.

use crate::perf::{
    AttemptKey, AttemptOrigin, AttemptRecord, CaptureRecord, InstrumentationFlags, LimitReport,
    attempt_json, capture_json,
};
use iron_canvas_core::chrome::{GridShape, PaneRegion};
use iron_canvas_core::renderer::diag::{DiagGeometry, DiagSegment, FrameDiagnostics};
use iron_canvas_core::{CanvasSize, RCRange};
use serde_json::{Value, json};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn diagnostics(attempt_seq: u64) -> FrameDiagnostics {
    FrameDiagnostics {
        schema_version: 3,
        attempt_seq,
        geometry: Some(DiagGeometry {
            canvas: CanvasSize { w: 640.0, h: 480.0 },
            backing_size: (800, 600),
            dpr: 1.25,
            sheet: 0,
            top_row: 1,
            left_column: 1,
            row_header_thickness: 40,
            col_header_thickness: 20,
            show_row_headers: true,
            show_col_headers: true,
            shape: GridShape::from_lens([0, 40], [0, 13]),
            segments: vec![DiagSegment {
                region: PaneRegion::BottomRight,
                range: RCRange {
                    r1: 1,
                    c1: 1,
                    r2: 40,
                    c2: 13,
                },
                cells: 520,
            }],
        }),
        ..FrameDiagnostics::default()
    }
}

/// Sheet names differ per attempt on purpose: a later rename must not rewrite
/// what an earlier record saw.
const SHEET_NAMES: [&str; 2] = ["Sheet1", "Renamed"];

fn record(attempt_seq: u64) -> AttemptRecord {
    AttemptRecord {
        key: AttemptKey {
            generation: 2,
            attempt_seq,
        },
        captured_at_ms: 1000.0 + attempt_seq as f64,
        render_call_ms: Some(3.5),
        origin: AttemptOrigin::Live,
        retry_of: None,
        batch_ids: vec![1],
        sheet_name: Some(SHEET_NAMES[attempt_seq as usize - 1].to_owned()),
        backing_size: (800, 600),
        diagnostics: diagnostics(attempt_seq),
    }
}

fn capture() -> CaptureRecord {
    CaptureRecord {
        id: 1,
        name: "capture-1".to_owned(),
        generation: 2,
        started_at_ms: 1000.0,
        completed_at_ms: Some(2000.0),
        pause_started_at_ms: None,
        paused: vec![(1200.0, 1300.0)],
        stop_reason: Some(crate::perf::StopReason::LimitReached(
            crate::perf::LimitKind::Attempts,
        )),
        instrumentation: InstrumentationFlags {
            paint_recording: true,
            forced_baselines: 1,
        },
        sheet_names: vec![(0, "Sheet1".to_owned())],
        attempts: vec![record(1), record(2)],
        batches: Vec::new(),
        mutations: Vec::new(),
        truncated: true,
        rejected_records: 1,
    }
}

fn limits() -> LimitReport {
    LimitReport {
        max_attempts: crate::perf::MAX_ATTEMPTS,
        max_batches: crate::perf::MAX_BATCHES,
        max_mutations: crate::perf::MAX_MUTATIONS,
        max_captures: crate::perf::MAX_CAPTURES,
        max_bytes: crate::perf::MAX_BYTES,
        retained_bytes: 4096,
        retained_captures: 2,
    }
}

#[wasm_bindgen_test]
fn capture_export_embeds_the_same_diagnostics_as_the_attempt_copy() {
    let source = capture();
    let envelope: Value =
        serde_json::from_str(&capture_json(&source, &limits(), 100.0).expect("export"))
            .expect("parses");

    assert_eq!(envelope["version"], json!(1));
    assert_eq!(envelope["diagnosticsSchemaVersion"], json!(3));
    assert_eq!(envelope["limits"]["retainedBytes"], json!(4096));
    assert_eq!(envelope["limits"]["retainedCaptures"], json!(2));
    assert_eq!(
        envelope["limits"]["maxAttempts"],
        json!(crate::perf::MAX_ATTEMPTS)
    );

    let capture = &envelope["capture"];
    assert_eq!(capture["id"], json!(1));
    assert_eq!(capture["name"], json!("capture-1"));
    assert_eq!(
        capture["stopReason"],
        json!({ "limitReached": "attempts" }),
        "the stop reason survives the envelope"
    );
    assert_eq!(capture["truncated"], json!(true));
    assert_eq!(capture["rejectedRecords"], json!(1));
    assert_eq!(capture["instrumentation"]["forcedBaselines"], json!(1));
    assert_eq!(capture["paused"][0], json!([1200.0, 1300.0]));
    assert_eq!(capture["sheetNames"][0], json!([0, "Sheet1"]));

    let attempts = capture["attempts"].as_array().expect("attempts");
    assert_eq!(attempts.len(), 2);
    for (index, attempt) in attempts.iter().enumerate() {
        assert_eq!(attempt["key"]["attemptSeq"], json!(index as u64 + 1));
        assert_eq!(attempt["key"]["generation"], json!(2));
        assert_eq!(attempt["backingSize"], json!([800, 600]));
        assert_eq!(attempt["sheetName"], json!(SHEET_NAMES[index]));

        // The embedded object matches the copy path field for field.
        let copied: Value =
            serde_json::from_str(&attempt_json(&source.attempts[index]).expect("attempt json"))
                .expect("parses");
        assert_eq!(
            attempt["diagnostics"], copied["diagnostics"],
            "attempt {index} embeds the projected snapshot"
        );
        assert_eq!(attempt["diagnostics"]["schemaVersion"], json!(3));
        assert_eq!(
            attempt["diagnostics"]["geometry"]["backingSize"]["w"],
            json!(800)
        );
        assert!(
            attempt["diagnostics"].is_object(),
            "diagnostics is an object, never an escaped string"
        );
    }
}

#[wasm_bindgen_test]
fn export_keeps_the_captured_sheet_identity_and_backing_size() {
    let envelope: Value =
        serde_json::from_str(&capture_json(&capture(), &limits(), 100.0).expect("export"))
            .expect("parses");
    let attempts = envelope["capture"]["attempts"]
        .as_array()
        .expect("attempts");

    assert_eq!(attempts[0]["sheetName"], json!("Sheet1"));
    assert_eq!(attempts[1]["sheetName"], json!("Renamed"));
    assert_eq!(attempts[1]["backingSize"], json!([800, 600]));
    assert_eq!(attempts[1]["renderCallMs"], json!(3.5));
    assert_eq!(attempts[1]["origin"], json!({ "kind": "live" }));
    assert_eq!(attempts[1]["batchIds"][0], json!(1));
    assert_eq!(attempts[1]["retryOf"], Value::Null);
}

/// A capture with no attempt still exports: the panel can download what it
/// has instead of refusing. `diagnosticsSchemaVersion` is zero because no
/// snapshot stated one.
///
/// The `ExportError::Projection` path has no test: it only fires when the
/// projection cannot be serialized, which `serde_json` cannot do for these
/// shapes. The variant stays because the caller must surface a failure rather
/// than write a partial file.
#[wasm_bindgen_test]
fn empty_capture_exports_an_envelope_with_no_attempts() {
    let mut empty = capture();
    empty.attempts.clear();
    let envelope: Value =
        serde_json::from_str(&capture_json(&empty, &limits(), 100.0).expect("export"))
            .expect("parses");
    assert_eq!(envelope["diagnosticsSchemaVersion"], json!(0));
    assert_eq!(envelope["capture"]["attempts"], json!([]));
    assert_eq!(
        envelope["capture"]["stopReason"],
        json!({ "limitReached": "attempts" })
    );
}

#[wasm_bindgen_test]
fn paused_export_keeps_its_open_interval_and_observation_time() {
    let mut archive = crate::perf::CaptureArchive::new();
    archive.request_start(false, 10.0, Vec::new()).unwrap();
    archive.confirm_start();
    archive.pause(20.0);
    let text = archive
        .with_active(|c| capture_json(c, &archive.limit_report(), 50.0).unwrap())
        .unwrap();
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["observedAtMs"], 50.0);
    assert_eq!(json["capture"]["pauseStartedAtMs"], 20.0);
    assert_eq!(json["capture"]["paused"], json!([]));
    assert_eq!(json["capture"]["completedAtMs"], Value::Null);
    archive.resume(false, 60.0).unwrap();
    archive
        .with_active(|c| {
            assert_eq!(c.pause_started_at_ms, None);
            assert_eq!(c.paused, vec![(20.0, 60.0)]);
        })
        .unwrap();
}
