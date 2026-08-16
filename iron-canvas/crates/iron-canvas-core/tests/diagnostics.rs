//! Native integration tests for the dev-diagnostics snapshot.
//! Harness mirrors tests/orchestrator_regimes.rs: MemSurface + TestModel.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegion;
use iron_canvas_core::{
    CanvasSize, DiagDeltaKind, Orchestrator, PaintResult, RCRange, RebuildReason,
};
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
#[test]
fn cold_start_reports_no_committed_frame_reason_and_delta_rebuild() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Rebuild));
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::NoCommittedFrame));
}

#[test]
fn freeze_rebuild_reports_reason_and_exact_segments() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_frozen_rows(3);
    orch.request_repaint();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::Freeze));
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    assert_eq!(geo.shape.frozen_rows(), 3);
    assert_eq!(geo.shape.frozen_cols(), 1);
    // Canonical TL/TR/BL/BR order, every region populated.
    let regions: Vec<PaneRegion> = geo.segments.iter().map(|s| s.region).collect();
    assert_eq!(
        regions,
        vec![
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight
        ]
    );
    // Frozen band: the TL/TR segments span rows 1..=3.
    assert_eq!(geo.segments[0].range.r2, 3);
    assert_eq!(geo.segments[1].range.r2, 3);
    assert_eq!(geo.segments[0].range.c2, 1);
    assert_eq!(geo.segments[2].range.c2, 1);
}

#[test]
fn probe_reports_exact_containing_segment_and_is_consumed() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    // Probe the frozen top-left corner: exactly TL contains it.
    orch.set_frame_diagnostics_probe(RCRange {
        r1: 1,
        c1: 1,
        r2: 1,
        c2: 1,
    });
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.probe,
        Some(RCRange {
            r1: 1,
            c1: 1,
            r2: 1,
            c2: 1
        })
    );
    assert_eq!(diag.probe_segments, vec![PaneRegion::TopLeft]);

    // The probe is attempt-scoped: the next attempt consumes nothing.
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.probe, None);
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn probe_outside_all_segments_reports_empty_attribution() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    orch.set_frame_diagnostics_probe(RCRange {
        r1: 999,
        c1: 999,
        r2: 999,
        c2: 999,
    });
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert!(diag.probe.is_some());
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn overlay_only_attempt_has_no_geometry_and_no_probe_segments() {
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
    assert_eq!(diag.delta, Some(DiagDeltaKind::Stable));
    assert!(diag.geometry.is_none());
    assert_eq!(diag.repaint.verdict, None);
    assert!(diag.probe_segments.is_empty());
}
