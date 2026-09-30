//! Committed link state: capture, transactional commit, and the metadata
//! escalation that keeps pixels and query data in step.
//!
//! The orchestrator is the only writer of committed link state, so these
//! tests drive real paint attempts and read the result back through the
//! public `link_at` query.

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{
    CanvasMetrics, CellLink, FrameInputFailure, LinkTarget, Orchestrator, PaintResult, RCRange,
    RenderStrategy,
};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

fn link(row: i32, column: i32, target: &str) -> CellLink {
    CellLink::new(
        RCRange::from_cell(row, column),
        LinkTarget::External(target.to_string()),
        None,
        false,
        None,
    )
}

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(
        CanvasMetrics::new(CanvasSize { w: 400.0, h: 300.0 }, 1.0)
            .expect("test canvas metrics are valid"),
    );
    orch.set_model(model);
    orch
}

fn model_with_links(links: Vec<CellLink>) -> Rc<TestModel> {
    Rc::new(TestModel::synthetic_grid().with_sheet_links(links))
}

#[test]
fn a_captured_link_list_commits_with_the_frame() {
    let model = model_with_links(vec![link(2, 3, "https://example.com")]);
    let mut orch = build(Rc::clone(&model));

    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let committed = orch
        .link_at(2, 3)
        .expect("the committed frame carries the captured link");
    assert_eq!(committed.target().as_str(), "https://example.com");
    assert!(orch.link_at(1, 1).is_none());
}

/// A failed capture holds the attempt, so the previously committed link state
/// stays readable — the new target is not observable.
#[test]
fn a_held_attempt_leaves_committed_link_state_in_place() {
    let model = model_with_links(vec![link(2, 3, "https://first.example")]);
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_sheet_links(vec![link(2, 3, "https://second.example")]);
    model.set_capture_fail(Some(FrameInputFailure::SheetLinks));
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);

    let committed = orch
        .link_at(2, 3)
        .expect("the held attempt must not clear committed link state");
    assert_eq!(committed.target().as_str(), "https://first.example");

    // With the read healthy again, the change commits.
    model.set_capture_fail(None);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the committed frame carries the link")
            .target()
            .as_str(),
        "https://second.example"
    );
}

/// A preparation failure *after* the link capture discards the whole
/// candidate, so a changed capture is not observable until an attempt commits
/// it — the committed index must be the previous one.
#[test]
fn a_preparation_failure_after_a_link_change_keeps_the_committed_index() {
    let model = model_with_links(vec![link(2, 3, "https://first.example")]);
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    // The link capture succeeds with the new target; the geometry walk that
    // follows it fails.
    model.set_sheet_links(vec![link(2, 3, "https://second.example")]);
    model.set_row_height_bridge_fail(true);
    model.set_frozen_rows(1);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the held attempt must not clear committed link state")
            .target()
            .as_str(),
        "https://first.example"
    );

    model.set_row_height_bridge_fail(false);
    model.set_frozen_rows(0);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the committed frame carries the link")
            .target()
            .as_str(),
        "https://second.example"
    );
}

/// A metadata-only link change must not take the overlay-only shortcut: the
/// link's underline and color are pixels, so the digest change forces
/// whole-grid content work even when nothing marked content dirty.
#[test]
fn a_metadata_only_link_change_escalates_to_content_work() {
    let model = model_with_links(vec![link(2, 3, "https://first.example")]);
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_sheet_links(vec![link(2, 3, "https://second.example")]);
    let grid_ops_before = orch.grid_surface().recorder().ops().len();
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::ChangedCells),
        "a changed link digest must escalate an overlay-only attempt to content work"
    );
    assert!(
        orch.grid_surface().recorder().ops().len() > grid_ops_before,
        "the escalation must repaint grid pixels, not only publish new query data"
    );
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the committed frame carries the link")
            .target()
            .as_str(),
        "https://second.example"
    );
}

/// A hold on the slots-reused grid path restores the previously committed link
/// index. `Chrome::next(.., FramePath::SlotsReuse)` refreshes the candidate's
/// index from the new capture, so the failed attempt already holds the new
/// target when the grid transaction holds — publishing it would show a link
/// whose pixels never committed.
#[test]
fn a_slots_reuse_hold_keeps_the_committed_link_index() {
    let model = model_with_links(vec![link(2, 3, "https://first.example")]);
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_sheet_links(vec![link(2, 3, "https://second.example")]);
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::ChangedCells),
        "the held attempt must be the slots-reused grid path"
    );
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the held attempt must not clear committed link state")
            .target()
            .as_str(),
        "https://first.example"
    );

    model.set_bulk_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the committed frame carries the link")
            .target()
            .as_str(),
        "https://second.example"
    );
}
