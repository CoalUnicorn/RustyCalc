//! Prepared/executed cell-envelope lifecycle and policy tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegion;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::{
    Border, BorderItem, BorderStyle, CanvasSize, CellStyle, DiagCacheResolution, DiagRepaintReason,
    FrameOutcome, GridVerdict, Orchestrator, PaintResult,
};
use iron_canvas_recorder::{DrawOp, MemSurface};

use common::TestModel;

fn harness(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    harness_at_dpr(model, 1.0)
}

fn harness_at_dpr(model: Rc<TestModel>, dpr: f64) -> Orchestrator<MemSurface> {
    let mut orchestrator = Orchestrator::new(MemSurface::new(), MemSurface::new());
    orchestrator.resize(CanvasSize { w: 800.0, h: 600.0 }, dpr);
    orchestrator.set_model(model);
    orchestrator.set_frame_diagnostics_enabled(true);
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    orchestrator
}

#[test]
fn unaligned_fractional_dpr_falls_back_to_full() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orchestrator = harness_at_dpr(Rc::clone(&model), std::f64::consts::SQRT_2);

    model.set_cell(5, 3, "changed");
    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert_eq!(diagnostics.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(
        diagnostics.repaint.reason,
        Some(DiagRepaintReason::ClipAlignment)
    );
}

fn visible_range(orchestrator: &Orchestrator<MemSurface>) -> iron_canvas_core::RCRange {
    orchestrator
        .frame_diagnostics()
        .expect("enabled diagnostics publish the baseline")
        .geometry
        .expect("the baseline visits grid geometry")
        .segments
        .into_iter()
        .find(|segment| segment.region == PaneRegion::BottomRight)
        .expect("the unfrozen fixture has a bottom-right segment")
        .range
}

fn cells_group(ops: &[DrawOp]) -> &[DrawOp] {
    let start = ops
        .iter()
        .position(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::Cells))
        .expect("a grid attempt opens the cells group")
        + 1;
    let end = ops[start..]
        .iter()
        .position(|op| matches!(op, DrawOp::EndGroup))
        .map(|offset| start + offset)
        .expect("a grid attempt closes the cells group");
    &ops[start..end]
}

#[test]
fn cell_envelope_wraps_all_contributor_paint() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orchestrator = harness(Rc::clone(&model));
    let cursor = orchestrator.grid_surface().recorder().ops().len();

    model.set_cell(5, 3, "changed");
    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    assert_eq!(orchestrator.last_trace().verdict, Some(GridVerdict::Cell));

    let ops = &orchestrator.grid_surface().recorder().ops()[cursor..];
    let cells = cells_group(ops);
    assert!(matches!(cells.first(), Some(DrawOp::PushClip { .. })));
    assert!(matches!(cells.last(), Some(DrawOp::PopClip)));
    let outer_fill = cells
        .iter()
        .position(|op| matches!(op, DrawOp::RectFill { .. }))
        .expect("the envelope clears its clip before contributor paint");
    assert!(outer_fill > 0);

    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert_eq!(
        diagnostics.repaint.reason,
        Some(DiagRepaintReason::ChangedCell)
    );
    assert_eq!(diagnostics.repaint.changed_cells.len(), 1);
    assert!(diagnostics.repaint.clip.is_some());
    assert!(diagnostics.paint_counts.cells < diagnostics.fetch.addressed_cells);
}

#[test]
fn every_bulk_channel_failure_holds_before_envelope_and_recovers() {
    for channel in 1..=4 {
        let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
        let mut orchestrator = harness(Rc::clone(&model));
        let ops_before = orchestrator.grid_surface().recorder().ops().len();
        let presents_before = orchestrator.grid_surface().presents();

        model.set_cell(5, 3, "changed");
        model.set_bulk_bridge_fail_channel(Some(channel));
        orchestrator.mark_content_dirty();
        assert_eq!(orchestrator.render_pending(), PaintResult::RetryRequired);
        assert_eq!(
            orchestrator.grid_surface().recorder().ops().len(),
            ops_before
        );
        assert_eq!(orchestrator.grid_surface().presents(), presents_before);
        let held = orchestrator.frame_diagnostics().unwrap();
        assert_eq!(held.outcome, FrameOutcome::HeldOnBridgeFailure);
        assert_eq!(held.cache.resolution, DiagCacheResolution::HeldForRetry);

        model.set_bulk_bridge_fail_channel(None);
        assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
        assert_eq!(orchestrator.last_trace().verdict, Some(GridVerdict::Cell));
        orchestrator.mark_content_dirty();
        assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
        assert_eq!(orchestrator.last_trace().verdict, Some(GridVerdict::Skip));
    }
}

#[test]
fn hidden_changed_cell_commits_without_pixel_work() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    model.set_row_height(4, 0.0);
    let mut orchestrator = harness(Rc::clone(&model));

    model.set_cell(4, 2, "hidden change");
    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert_eq!(diagnostics.repaint.verdict, Some(GridVerdict::Cell));
    assert_eq!(diagnostics.repaint.clip, None);
    assert!(diagnostics.repaint.source_ranges.is_empty());
    assert_eq!(diagnostics.paint_counts.rows, 0);
    assert_eq!(diagnostics.paint_counts.cells, 0);

    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    assert_eq!(orchestrator.last_trace().verdict, Some(GridVerdict::Skip));
}

#[test]
fn sparse_wide_changes_choose_the_cheaper_row_sweep() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orchestrator = harness(Rc::clone(&model));
    let range = visible_range(&orchestrator);
    model.set_cell(range.r1 + 2, range.c1 + 2, "first");
    model.set_cell(range.r2 - 2, range.c2 - 2, "second");

    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert!(matches!(
        diagnostics.repaint.verdict,
        Some(GridVerdict::Rows { .. })
    ));
    assert_eq!(
        diagnostics.repaint.reason,
        Some(DiagRepaintReason::ChangedRows)
    );
    assert_eq!(diagnostics.repaint.changed_cells.len(), 2);
    assert_eq!(diagnostics.repaint.clip, None);
}

#[test]
fn unsafe_row_boundary_selects_the_merged_range() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orchestrator = harness(Rc::clone(&model));
    let range = visible_range(&orchestrator);
    let first = (range.r1 + 2, range.c1 + 2);
    let second = (range.r2 - 2, range.c2 - 2);
    model.set_style(
        first.0,
        first.1,
        CellStyle {
            border: Border {
                bottom: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    model.set_cell(first.0, first.1, "first");
    model.set_cell(second.0, second.1, "second");

    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    assert_eq!(orchestrator.last_trace().verdict, Some(GridVerdict::Range));
    assert_eq!(
        orchestrator.frame_diagnostics().unwrap().repaint.reason,
        Some(DiagRepaintReason::ChangedCells)
    );
}

#[test]
fn whole_visible_bounding_range_degenerates_to_full() {
    let model = Rc::new(TestModel::synthetic_grid().with_data_until(40));
    let mut orchestrator = harness(Rc::clone(&model));
    let range = visible_range(&orchestrator);
    model.set_cell(range.r1, range.c1, "first");
    model.set_cell(range.r2, range.c2, "second");

    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert_eq!(diagnostics.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(
        diagnostics.repaint.reason,
        Some(DiagRepaintReason::ChangedCells)
    );
    assert_eq!(diagnostics.repaint.changed_cells.len(), 2);
    assert_eq!(diagnostics.repaint.clip, None);
}

#[test]
fn frozen_envelope_sources_keep_canonical_region_order() {
    let model = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(40)
            .with_frozen(2, 2),
    );
    let mut orchestrator = harness(Rc::clone(&model));
    model.set_cell(2, 2, "seam change");

    orchestrator.mark_content_dirty();
    assert_eq!(orchestrator.render_pending(), PaintResult::Rendered);
    let diagnostics = orchestrator.frame_diagnostics().unwrap();
    assert_eq!(diagnostics.repaint.verdict, Some(GridVerdict::Cell));
    let order: Vec<usize> = diagnostics
        .repaint
        .source_ranges
        .iter()
        .map(|source| source.region.index())
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        diagnostics
            .repaint
            .source_ranges
            .first()
            .map(|source| source.region),
        Some(PaneRegion::TopLeft)
    );
}
