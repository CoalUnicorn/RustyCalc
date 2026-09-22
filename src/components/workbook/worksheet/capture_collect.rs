//! Turning one canvas snapshot into a retained attempt record.
//!
//! Both collectors — the render loop and the recorder's forced baseline — go
//! through here, so the ordering rule has one home: the retry link and the
//! claimed host batches are read *before* the record is appended, and a forced
//! baseline claims neither, because a tool action preceded it rather than host
//! input.

use leptos::prelude::*;

use crate::app_state::AppState;
use crate::model::SheetRoster;
use crate::perf::{
    AppendOutcome, AttemptKey, AttemptOrigin, AttemptRecord, CaptureState, PerfStore,
};
use crate::state::ModelStore;
use iron_canvas_web::{CanvasFrameSnapshot, IronCanvas};

/// Resolve a sheet name through the model roster, at capture time. A later
/// rename must not rewrite what a record saw.
pub(super) fn sheet_name(model: ModelStore, sheet: u32) -> Option<String> {
    model.with_value(|model| {
        model
            .get_sheet_names()
            .into_iter()
            .find(|(id, _)| *id == sheet)
            .map(|(_, name)| name)
    })
}

/// Build the record for one snapshot, claiming what the origin allows.
fn attempt_record(
    store: &PerfStore,
    model: ModelStore,
    snapshot: CanvasFrameSnapshot,
    origin: AttemptOrigin,
    render_call_ms: Option<f64>,
) -> AttemptRecord {
    let (retry_of, batch_ids) = match origin {
        AttemptOrigin::ForcedBaseline => (None, Vec::new()),
        AttemptOrigin::Live => (store.retry_link(), store.claim_pending_batches()),
    };
    let diagnostics = snapshot.diagnostics;
    let sheet_name = diagnostics
        .geometry
        .as_ref()
        .and_then(|geometry| sheet_name(model, geometry.sheet));
    AttemptRecord {
        key: AttemptKey {
            generation: store.generation(),
            attempt_seq: diagnostics.attempt_seq,
        },
        captured_at_ms: crate::perf::now(),
        render_call_ms,
        origin,
        retry_of,
        batch_ids,
        sheet_name,
        backing_size: snapshot.backing_size,
        diagnostics,
    }
}

/// Append a snapshot without consuming provenance for a duplicate. Update the
/// retry predecessor for both live attempts and recorder baselines.
pub(super) fn append_snapshot(
    store: &PerfStore,
    model: ModelStore,
    snapshot: CanvasFrameSnapshot,
    origin: AttemptOrigin,
    render_call_ms: Option<f64>,
) -> AppendOutcome {
    let key = AttemptKey {
        generation: store.generation(),
        attempt_seq: snapshot.diagnostics.attempt_seq,
    };
    if store.with_active(|capture| capture.attempts.iter().any(|a| a.key == key)) == Some(true) {
        return AppendOutcome::Duplicate;
    }
    let held = !matches!(
        snapshot.diagnostics.outcome,
        iron_canvas_core::FrameOutcome::Painted
    );
    let record = attempt_record(store, model, snapshot, origin, render_call_ms);
    let outcome = store.append_attempt(record);
    if outcome == AppendOutcome::Appended {
        store.remember_held(key, held);
    }
    outcome
}

/// Retain the recorder's forced baseline paint.
///
/// The recorder forces one full paint when recording starts, and that paint is
/// the replay anchor. Retaining its diagnostics gives the capture the same
/// reference frame. Deduplication by key handles the render loop reporting the
/// same attempt later.
pub(super) fn append_forced_baseline(app: &AppState, model: ModelStore, canvas: &mut IronCanvas) {
    let store = app.perf_store;
    if !matches!(store.state_untracked(), CaptureState::Capturing(_)) {
        return;
    }
    let Some(snapshot) = canvas.frame_diagnostics_snapshot() else {
        return;
    };
    append_snapshot(&store, model, snapshot, AttemptOrigin::ForcedBaseline, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ContentEvent, SpreadsheetEvent};
    use iron_canvas_core::{FrameDiagnostics, FrameOutcome};
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn baseline_updates_retry_memory_and_duplicates_preserve_batches() {
        Owner::new().with(|| {
            let store = PerfStore::new();
            store.request_start(false, Vec::new()).unwrap();
            store.confirm_start();
            let model = StoredValue::new_local(
                ironcalc_base::UserModel::new_empty("Sheet1", "en", "UTC", "en").unwrap(),
            );
            let snapshot = |attempt_seq, outcome| CanvasFrameSnapshot {
                diagnostics: FrameDiagnostics {
                    attempt_seq,
                    outcome,
                    ..Default::default()
                },
                backing_size: (100, 100),
            };
            let append = |seq, outcome, origin| {
                append_snapshot(&store, model, snapshot(seq, outcome), origin, None)
            };
            append(1, FrameOutcome::HeldOnBridgeFailure, AttemptOrigin::Live);
            append(
                2,
                FrameOutcome::HeldOnBridgeFailure,
                AttemptOrigin::ForcedBaseline,
            );
            store.note_batch(
                7,
                &[SpreadsheetEvent::Content(ContentEvent::NamedRangesChanged)],
                |_| None,
            );
            assert_eq!(
                append(2, FrameOutcome::HeldOnBridgeFailure, AttemptOrigin::Live),
                AppendOutcome::Duplicate
            );
            append(3, FrameOutcome::Painted, AttemptOrigin::Live);
            store
                .with_active(|capture| {
                    assert_eq!(capture.attempts[1].retry_of, None);
                    assert!(capture.attempts[1].batch_ids.is_empty());
                    assert_eq!(capture.attempts[2].retry_of, Some(capture.attempts[1].key));
                    assert_eq!(capture.attempts[2].batch_ids, vec![7]);
                })
                .unwrap();
            append(4, FrameOutcome::HeldOnBridgeFailure, AttemptOrigin::Live);
            append(5, FrameOutcome::Painted, AttemptOrigin::ForcedBaseline);
            append(6, FrameOutcome::Painted, AttemptOrigin::Live);
            store
                .with_active(|c| assert_eq!(c.attempts.last().unwrap().retry_of, None))
                .unwrap();
        });
    }
}
