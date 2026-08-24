//! Whole-grid transactional hold and retry integration tests.
//!
//! A bridge failure holds every grid segment together: no cache candidate,
//! geometry, draw operation, or presentation may commit. Recovery is driven
//! entirely by retained work and needs no new host notification.

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{
    FrameInputFailure, FrameOutcome, GridVerdict, Orchestrator, PaintRegimeTag, PaintResult,
    RowSpan, WorkFlags,
};
use iron_canvas_recorder::{DrawOp, MemSurface};

use common::TestModel;

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_model(model);
    orch
}

fn grid_ops_len(orch: &Orchestrator<MemSurface>) -> usize {
    orch.grid_surface().recorder().ops().len()
}

fn overlay_ops_len(orch: &Orchestrator<MemSurface>) -> usize {
    orch.overlay_surface().recorder().ops().len()
}

fn grid_text_ops_containing(orch: &Orchestrator<MemSurface>, needle: &str) -> usize {
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::FillText { text, .. } if text.contains(needle)))
        .count()
}

#[test]
fn input_capture_hold_resets_renderer_trace_before_capture() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(orch.last_trace().fetched_cells > 0);

    model.set_capture_fail(Some(FrameInputFailure::SelectedSheet));
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);

    let trace = orch.last_trace();
    assert_eq!(trace.attempt_seq, 2);
    assert_eq!(trace.committed_seq, None);
    assert_eq!(trace.regime, None);
    assert_eq!(trace.effective, None);
    assert_eq!(trace.verdict, None);
    assert_eq!(trace.fetched_cell_slots, 0);
    assert_eq!(trace.fetched_cells, 0);
    assert_eq!(trace.fetch_batches, 0);
    assert_eq!(trace.blit_fallback, None);
    assert_eq!(
        trace.outcome,
        FrameOutcome::HeldOnInputFailure(FrameInputFailure::SelectedSheet)
    );
}

fn scroll_then_fail(model: &TestModel, orch: &mut Orchestrator<MemSurface>) -> PaintResult {
    model.set_top_row(2);
    model.set_bulk_bridge_fail(true);
    orch.view_changed();
    orch.paint_if_dirty()
}

#[test]
fn held_viewport_rolls_back_everything_and_recovery_commits_exact_history() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let rect_before = orch.cell_rect(1, 1);
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    assert_eq!(scroll_then_fail(&model, &mut orch), PaintResult::Retry);
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(overlay_ops_len(&orch), overlay_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.overlay_surface().presents(), overlay_presents);
    assert_eq!(orch.cell_rect(1, 1), rect_before);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(orch.last_trace().effective, None);

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "a bridge retry widens to whole-grid content, so content plus the retained scroll is Fresh"
    );
    assert!(orch.cell_rect(1, 1).is_none());

    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Skip));
}

#[test]
fn held_frame_grid() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2)
            .with_show_selection(false),
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    model.set_cell(1, 3, "frozen-edit");
    model.set_cell(6, 3, "scroll-edit");
    model.set_active(2, 3);
    model.set_bulk_bridge_fail_from(Some(3));
    orch.mark_content_dirty();
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "healthy segments must not leak"
    );
    assert_eq!(overlay_ops_len(&orch), overlay_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.overlay_surface().presents(), overlay_presents);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);

    model.set_bulk_bridge_fail_from(None);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
    assert_eq!(
        orch.last_work_flags(),
        WorkFlags::VIEW | WorkFlags::CONTENT | WorkFlags::OVERLAY
    );
    assert!(grid_text_ops_containing(&orch, "frozen-edit") > 0);
    assert!(grid_text_ops_containing(&orch, "scroll-edit") > 0);
}

#[test]
fn held_damage_is_whole_grid_and_retries_grid_wide() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    model.set_cell(1, 2, "frozen-damage");
    model.set_cell(6, 2, "scroll-damage");
    model.set_bulk_bridge_fail_from(Some(3));
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 });
    orch.mark_rows_damaged(0, RowSpan { r1: 6, r2: 6 });

    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "damage must commit atomically"
    );
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);

    model.set_bulk_bridge_fail_from(None);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::SlotsReuse),
        "a bridge retry widens the original rows to whole-grid content"
    );
    assert!(grid_text_ops_containing(&orch, "frozen-damage") > 0);
    assert!(grid_text_ops_containing(&orch, "scroll-damage") > 0);
}

#[test]
fn held_fresh_content_plus_scroll_keeps_committed_geometry_until_recovery() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let rect_before = orch.cell_rect(1, 1);
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    model.set_cell(5, 1, "edited");
    model.set_top_row(5);
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(overlay_ops_len(&orch), overlay_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.overlay_surface().presents(), overlay_presents);
    assert_eq!(orch.cell_rect(1, 1), rect_before);

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(orch.cell_rect(1, 1).is_none());
    assert!(grid_text_ops_containing(&orch, "edited") > 0);
}

#[test]
fn held_first_fresh_attempt_has_no_visible_or_query_state() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    model.set_bulk_bridge_fail(true);
    let mut orch = build(Rc::clone(&model));
    let grid_ops = grid_ops_len(&orch);

    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    assert_eq!(orch.cell_rect(1, 1), None);
    assert_eq!(orch.grid_surface().presents(), 0);
    assert_eq!(orch.overlay_surface().presents(), 0);
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(orch.cell_rect(1, 1).is_some());
}

#[test]
fn new_work_merges_with_retained_whole_grid_retry() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_cell(6, 3, "held-edit");
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);

    model.set_bulk_bridge_fail(false);
    model.set_cell(1, 3, "late-edit");
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(grid_text_ops_containing(&orch, "held-edit") > 0);
    assert!(grid_text_ops_containing(&orch, "late-edit") > 0);
}
