//! Whole-grid transactional hold and retry integration tests.
//!
//! A bridge failure holds every grid segment together: no cache candidate,
//! geometry, draw operation, or presentation may commit. Recovery is driven
//! entirely by retained work and needs no new host notification.

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{
    FrameInputFailure, FrameOutcome, GridVerdict, Orchestrator, PaintResult, RenderStrategy,
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
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.last_trace().fetched_cells > 0);

    model.set_capture_fail(Some(FrameInputFailure::SelectedSheet));
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);

    let trace = orch.last_trace();
    assert_eq!(trace.attempt_seq, 2);
    assert_eq!(trace.committed_seq, None);
    assert_eq!(trace.strategy, None);
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
    orch.render_pending()
}

#[test]
fn held_viewport_rolls_back_everything_and_recovery_commits_exact_history() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let rect_before = orch.cell_rect(1, 1);
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    assert_eq!(
        scroll_then_fail(&model, &mut orch),
        PaintResult::RetryRequired
    );
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(overlay_ops_len(&orch), overlay_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.overlay_surface().presents(), overlay_presents);
    assert_eq!(orch.cell_rect(1, 1), rect_before);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(orch.last_trace().effective, None);

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::FullRebuild),
        "a bridge retry widens to whole-grid content, so content plus the retained scroll is Fresh"
    );
    assert!(orch.cell_rect(1, 1).is_none());

    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
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
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

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

    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
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
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(orch.last_strategy(), Some(RenderStrategy::ChangedCells));
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
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    model.set_cell(1, 2, "frozen-damage");
    model.set_cell(6, 2, "scroll-damage");
    model.set_bulk_bridge_fail_from(Some(3));
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 });
    orch.mark_rows_damaged(0, RowSpan { r1: 6, r2: 6 });

    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "damage must commit atomically"
    );
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);

    model.set_bulk_bridge_fail_from(None);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::ChangedCells),
        "a bridge retry widens the original rows to whole-grid content"
    );
    assert!(grid_text_ops_containing(&orch, "frozen-damage") > 0);
    assert!(grid_text_ops_containing(&orch, "scroll-damage") > 0);
}

#[test]
fn held_fresh_content_plus_scroll_keeps_committed_geometry_until_recovery() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

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

    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(orch.last_strategy(), Some(RenderStrategy::FullRebuild));
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(overlay_ops_len(&orch), overlay_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);
    assert_eq!(orch.overlay_surface().presents(), overlay_presents);
    assert_eq!(orch.cell_rect(1, 1), rect_before);

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.cell_rect(1, 1).is_none());
    assert!(grid_text_ops_containing(&orch, "edited") > 0);
}

#[test]
fn held_first_fresh_attempt_has_no_visible_or_query_state() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    model.set_bulk_bridge_fail(true);
    let mut orch = build(Rc::clone(&model));
    let grid_ops = grid_ops_len(&orch);

    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(orch.cell_rect(1, 1), None);
    assert_eq!(orch.grid_surface().presents(), 0);
    assert_eq!(orch.overlay_surface().presents(), 0);
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(orch.last_trace().verdict, Some(GridVerdict::Held));

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.cell_rect(1, 1).is_some());
}

#[test]
fn new_work_merges_with_retained_whole_grid_retry() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_cell(6, 3, "held-edit");
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);

    model.set_bulk_bridge_fail(false);
    model.set_cell(1, 3, "late-edit");
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(grid_text_ops_containing(&orch, "held-edit") > 0);
    assert!(grid_text_ops_containing(&orch, "late-edit") > 0);
}

#[test]
fn invalid_frozen_row_count_holds_before_any_geometry() {
    let model = Rc::new(TestModel::synthetic_grid());
    model.set_frozen_rows(-1);
    let mut orch = build(Rc::clone(&model));
    // `build` leaves both recorders with their static scene-setup ops only.
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);

    // A negative frozen count must hold at capture, before `Vec::reserve`
    // or the frozen-band walk: no new geometry, draw ops, or presentation.
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(
        orch.last_trace().outcome,
        FrameOutcome::HeldOnInputFailure(FrameInputFailure::InvalidFrozenRowCount)
    );
    assert_eq!(orch.cell_rect(1, 1), None);
    assert_eq!(orch.grid_surface().presents(), 0);
    assert_eq!(orch.overlay_surface().presents(), 0);
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(overlay_ops_len(&orch), overlay_ops);

    model.set_frozen_rows(0);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.cell_rect(1, 1).is_some());
    assert!(grid_ops_len(&orch) > grid_ops, "recovery paints cells");
}

#[test]
fn invalid_frozen_counts_preserve_committed_frame_and_retry_work() {
    let cases = [
        (-1, 0, FrameInputFailure::InvalidFrozenRowCount),
        (
            iron_canvas_core::LAST_ROW + 1,
            0,
            FrameInputFailure::InvalidFrozenRowCount,
        ),
        (0, -1, FrameInputFailure::InvalidFrozenColumnCount),
        (
            0,
            iron_canvas_core::LAST_COLUMN + 1,
            FrameInputFailure::InvalidFrozenColumnCount,
        ),
    ];

    for (rows, cols, failure) in cases {
        let model = Rc::new(TestModel::synthetic_grid());
        let mut orch = build(Rc::clone(&model));
        assert_eq!(orch.render_pending(), PaintResult::Rendered);
        let rect = orch.cell_rect(1, 1).expect("initial frame contains A1");
        let grid_ops = grid_ops_len(&orch);
        let overlay_ops = overlay_ops_len(&orch);
        let grid_presents = orch.grid_surface().presents();
        let overlay_presents = orch.overlay_surface().presents();

        model.set_cell(1, 1, "retry-after-invalid-freeze");
        model.set_frozen_rows(rows);
        model.set_frozen_cols(cols);
        orch.mark_content_dirty();
        for _ in 0..2 {
            assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
            assert_eq!(
                orch.last_trace().outcome,
                FrameOutcome::HeldOnInputFailure(failure)
            );
            assert_eq!(orch.last_trace().committed_seq, None);
            assert_eq!(orch.cell_rect(1, 1), Some(rect));
            assert_eq!(grid_ops_len(&orch), grid_ops);
            assert_eq!(overlay_ops_len(&orch), overlay_ops);
            assert_eq!(orch.grid_surface().presents(), grid_presents);
            assert_eq!(orch.overlay_surface().presents(), overlay_presents);
        }

        model.set_frozen_rows(0);
        model.set_frozen_cols(0);
        assert_eq!(orch.render_pending(), PaintResult::Rendered);
        assert!(grid_text_ops_containing(&orch, "retry-after-invalid-freeze") > 0);
        assert_eq!(orch.render_pending(), PaintResult::Idle);
    }
}

// ─── FSM-A2: geometry and configuration bridge failures ──────────────────
//
// Row heights, column widths, and grid-line visibility now carry an explicit
// fetch outcome. A transient `BridgeFailed` on any of them must hold the
// whole attempt before paint — no fabricated default geometry or grid state
// may commit — and recovery needs no new host notification, exactly like the
// content-bridge holds above.

/// A geometry read failure during `Chrome::build` (fresh geometry) must hold
/// the attempt before any paint, preserving the committed frame, draw ops,
/// and presentations. The edited content stays queued; it renders once the
/// read recovers.
///
/// A freeze toggle forces the Fresh geometry walk (SlotsReuse strategies
/// never re-read row heights). A1 stays visible in the frozen band, so the
/// committed rect must survive every held retry unchanged.
#[test]
fn row_height_bridge_failure_holds_fresh_geometry_and_retries() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let rect = orch.cell_rect(1, 1).expect("A1 visible before failure");

    model.set_cell(1, 1, "row-height-retry");
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let content_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();

    model.set_row_height_bridge_fail(true);
    model.set_frozen_rows(1); // forces Fresh: the row walk runs
    orch.view_changed();
    for _ in 0..2 {
        assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
        assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
        assert_eq!(orch.last_trace().committed_seq, None);
        assert_eq!(orch.cell_rect(1, 1), Some(rect));
        assert_eq!(grid_ops_len(&orch), content_ops);
        assert_eq!(orch.grid_surface().presents(), grid_presents);
    }

    model.set_row_height_bridge_fail(false);
    model.set_frozen_rows(0); // another Fresh rebuild now succeeds
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(grid_text_ops_containing(&orch, "row-height-retry") > 0);
    assert_eq!(orch.render_pending(), PaintResult::Idle);
}

/// Column widths feed the same fresh-geometry walk; a `BridgeFailed` there
/// holds exactly like a row-height failure.
#[test]
fn col_width_bridge_failure_holds_fresh_geometry_and_retries() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let rect = orch.cell_rect(1, 1).expect("A1 visible before failure");
    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();

    model.set_col_width_bridge_fail(true);
    model.set_frozen_cols(1); // forces Fresh: the col walk runs
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(orch.last_trace().committed_seq, None);
    assert_eq!(orch.cell_rect(1, 1), Some(rect));
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);

    model.set_col_width_bridge_fail(false);
    model.set_frozen_cols(0);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert!(orch.cell_rect(1, 1).is_some(), "committed view rebuilt");
    assert_eq!(orch.render_pending(), PaintResult::Idle);
}

/// Grid-line visibility is per-execution config: `BridgeFailed` must hold
/// the grid transaction (no fabricated show/hide state painted), then
/// recover with the model's real answer.
#[test]
fn grid_lines_bridge_failure_holds_before_paint_and_recovers() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(20));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();

    model.set_grid_lines_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);

    model.set_grid_lines_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(orch.render_pending(), PaintResult::Idle);
}

/// A host extent value that cannot become geometry (NaN row height) must
/// hold exactly like a bridge failure: no fabricated hidden row from an
/// `as i32` cast, no committed geometry. Recovery is a fixed host value.
#[test]
fn invalid_row_height_holds_fresh_geometry_and_retries() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let rect = orch.cell_rect(1, 1).expect("A1 visible before failure");
    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();

    model.set_row_height(1, f64::NAN); // a Value that cannot become px
    model.set_frozen_rows(1); // force Fresh: the row walk reads row 1
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(orch.last_trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(orch.last_trace().committed_seq, None);
    assert_eq!(orch.cell_rect(1, 1), Some(rect));
    assert_eq!(grid_ops_len(&orch), grid_ops);
    assert_eq!(orch.grid_surface().presents(), grid_presents);

    model.set_row_height(1, 20.0);
    model.set_frozen_rows(0);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(orch.render_pending(), PaintResult::Idle);
}

#[test]
fn extent_coordinate_overflow_holds_and_recovers_without_notification() {
    for rows in [true, false] {
        for frozen in [0, 2] {
            let model = Rc::new(TestModel::synthetic_grid());
            let mut orch = build(Rc::clone(&model));
            assert_eq!(orch.render_pending(), PaintResult::Rendered);
            let rect = orch.cell_rect(1, 1);
            let grid_ops = grid_ops_len(&orch);
            let overlay_ops = overlay_ops_len(&orch);
            let presents = (
                orch.grid_surface().presents(),
                orch.overlay_surface().presents(),
            );

            // Each extent fits i32. The cursor plus one extent (scroll)
            // or the sum of two extents (frozen) does not.
            let extent = if frozen == 0 { i32::MAX } else { i32::MAX / 2 };
            for id in 1..=2 {
                if rows {
                    model.set_row_height(id, f64::from(extent));
                } else {
                    model.set_col_width(id, f64::from(extent));
                }
            }
            model.set_frozen_rows(if rows { frozen } else { 0 });
            model.set_frozen_cols(if rows { 0 } else { frozen });
            orch.request_repaint();
            for _ in 0..2 {
                assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
                assert_eq!(orch.cell_rect(1, 1), rect);
                assert_eq!(grid_ops_len(&orch), grid_ops);
                assert_eq!(overlay_ops_len(&orch), overlay_ops);
                assert_eq!(
                    (
                        orch.grid_surface().presents(),
                        orch.overlay_surface().presents()
                    ),
                    presents
                );
                assert_eq!(orch.last_trace().committed_seq, None);
            }
            for id in 1..=2 {
                model.set_row_height(id, 20.0);
                model.set_col_width(id, 80.0);
            }
            assert_eq!(orch.render_pending(), PaintResult::Rendered);
            assert_eq!(orch.render_pending(), PaintResult::Idle);
        }
    }
}
