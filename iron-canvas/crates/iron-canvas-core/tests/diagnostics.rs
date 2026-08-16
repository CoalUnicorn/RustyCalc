//! Native integration tests for the dev-diagnostics snapshot.
//! Harness mirrors tests/orchestrator_regimes.rs: MemSurface + TestModel.

mod common;

use std::rc::Rc;

use iron_canvas_core::{CanvasSize, Orchestrator, PaintResult};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

fn harness() -> (Orchestrator<MemSurface>, Rc<TestModel>) {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    (orch, model)
}

#[test]
fn disabled_by_default_publishes_no_snapshot() {
    let (mut orch, _model) = harness();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn enable_then_disable_round_trips() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().expect("enabled capture publishes");
    assert_eq!(diag.schema_version, 1);
    assert_eq!(diag.attempt_seq, 1);
    assert_eq!(diag.committed_seq, Some(1));
    orch.set_frame_diagnostics_enabled(false);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn capture_hold_still_publishes_and_keeps_cache_state() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_capture_fail(Some(iron_canvas_core::FrameInputFailure::SelectedSheet));
    orch.request_repaint();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.attempt_seq, 2);
    assert_eq!(diag.committed_seq, None);
    assert_eq!(
        diag.outcome,
        iron_canvas_core::FrameOutcome::HeldOnInputFailure(
            iron_canvas_core::FrameInputFailure::SelectedSheet
        )
    );
    assert_eq!(
        diag.cache.committed_before,
        Some(diag.cache.committed_after.clone()),
        "a held attempt never presents changed cache state"
    );
}

#[test]
fn overlay_only_attempt_commits_without_cache_work() {
    // Live recipe from orchestrator_regimes.rs:1303-1324: an in-viewport
    // selection move is a committed Overlay regime with NO grid cache
    // commit. It must not be mislabelled as held.
    let model = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_active(6, 2);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.cache.resolution,
        iron_canvas_core::DiagCacheResolution::Committed,
        "an Overlay regime commits a transaction; it is not held"
    );
    assert!(diag.committed_seq.is_some());
    assert_eq!(diag.cache.planned_action, None);
    assert!(!diag.painted_layers.grid);
    assert!(diag.painted_layers.overlay);
    assert_eq!(
        diag.cache.committed_before,
        Some(diag.cache.committed_after.clone())
    );
    assert!(diag.geometry.is_none());
}
