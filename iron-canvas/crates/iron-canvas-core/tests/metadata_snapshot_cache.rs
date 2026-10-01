//! Metadata snapshot reuse: the orchestrator reuses the last validated
//! `LinkIndex`/`MergeTable` when the host reports an unchanged metadata epoch
//! for the same model and sheet, and rebuilds them otherwise.
//!
//! `TestModel::metadata_list_calls` counts the two whole-list bridge reads
//! (`get_sheet_links` + `get_merged_ranges`), so these tests observe reuse
//! directly. They also pin the safety half: an epoch move that carries a
//! changed link must still rebuild and escalate past the overlay-only path.

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

fn merged(r1: i32, c1: i32, r2: i32, c2: i32) -> RCRange {
    RCRange { r1, c1, r2, c2 }
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

fn model_with_metadata() -> Rc<TestModel> {
    Rc::new(
        TestModel::synthetic_grid()
            .with_sheet_links(vec![link(2, 3, "https://example.com")])
            .with_merged_ranges(vec![merged(5, 5, 6, 6)]),
    )
}

/// A first attempt reads both lists once each. An overlay-only repaint that
/// keeps the same epoch reuses the snapshot: no further bridge reads, and the
/// committed link is still queryable.
#[test]
fn an_unchanged_epoch_reuses_the_metadata_snapshot() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));

    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let reads = model.metadata_list_calls();
    assert_eq!(reads, 2, "the first attempt reads links and merges once");

    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        model.metadata_list_calls(),
        reads,
        "an unchanged epoch must not re-read the link or merge list"
    );
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the reused snapshot stays committed")
            .target()
            .as_str(),
        "https://example.com"
    );
}

/// Advancing the epoch forces a rebuild: the bridge is read again, and a link
/// that changed under the new epoch still escalates past the overlay-only
/// shortcut and commits.
#[test]
fn an_advanced_epoch_rebuilds_and_publishes_a_changed_link() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let reads = model.metadata_list_calls();

    model.set_sheet_links(vec![link(2, 3, "https://second.example")]);
    orch.set_metadata_epoch(Some(1));
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    assert_eq!(
        model.metadata_list_calls(),
        reads + 2,
        "an advanced epoch must re-read both lists"
    );
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::ChangedCells),
        "the rebuilt link digest must escalate the overlay attempt to content work"
    );
    assert_eq!(
        orch.link_at(2, 3)
            .expect("the rebuilt snapshot is committed")
            .target()
            .as_str(),
        "https://second.example"
    );
}

/// Without a host epoch, the engine cannot tell "unchanged" from "changed", so
/// every attempt re-reads — the pre-cache behavior that keeps a host which
/// cannot observe metadata changes correct.
#[test]
fn a_missing_epoch_always_rebuilds() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(model.metadata_list_calls(), 2);

    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        model.metadata_list_calls(),
        4,
        "a None epoch disables reuse, so the second attempt re-reads"
    );
}

#[test]
fn disabling_reuse_discards_the_previous_snapshot() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    orch.set_metadata_epoch(None);
    model.set_sheet_links(vec![link(2, 3, "https://current.example")]);
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(model.metadata_list_calls(), 4);

    // Revision tracking starts again after an interval without a revision.
    orch.set_metadata_epoch(Some(0));
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.link_at(2, 3).expect("current link").target().as_str(),
        "https://current.example",
        "re-enabling reuse must not restore metadata from before None"
    );
    assert_eq!(model.metadata_list_calls(), 6);
}

/// A model replacement drops the cached snapshot even when the host reuses the
/// same epoch value: the new model's lists are unrelated to the old model's.
#[test]
fn a_model_replacement_drops_the_snapshot() {
    let first = model_with_metadata();
    let mut orch = build(Rc::clone(&first));
    orch.set_metadata_epoch(Some(7));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(first.metadata_list_calls(), 2);

    let second = Rc::new(TestModel::synthetic_grid().with_sheet_links(vec![link(
        4,
        4,
        "https://new.example",
    )]));
    orch.set_model(second.clone());
    orch.set_metadata_epoch(Some(7));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    assert_eq!(
        second.metadata_list_calls(),
        2,
        "the replacement model's lists must be read, not inherited"
    );
    assert_eq!(
        orch.link_at(4, 4)
            .expect("the replacement model's link is committed")
            .target()
            .as_str(),
        "https://new.example"
    );
}

#[test]
fn a_sheet_change_rebuilds_with_the_same_epoch() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_sheet(1);
    model.set_sheet_links(vec![link(2, 3, "https://sheet-two.example")]);
    orch.view_changed();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(model.metadata_list_calls(), 4);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("second sheet link")
            .target()
            .as_str(),
        "https://sheet-two.example"
    );
}

#[test]
fn an_advanced_epoch_publishes_changed_merge_geometry() {
    let model = Rc::new(TestModel::synthetic_grid().with_top_row(1));
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let cell = orch.cell_rect(2, 3).expect("visible cell");
    let point = (f64::from(cell.left() + 1), f64::from(cell.top() + 1));

    model.set_merged_ranges(vec![merged(2, 2, 3, 3)]);
    orch.set_metadata_epoch(Some(1));
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(orch.last_strategy(), Some(RenderStrategy::FullRebuild));
    assert_eq!(model.metadata_list_calls(), 4);
    assert_eq!(
        orch.display_cell_at(point.0, point.1)
            .expect("merged cell")
            .merged,
        merged(2, 2, 3, 3)
    );
}

#[test]
fn failed_metadata_capture_does_not_cache_a_partial_pair() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    model.set_sheet_links(vec![link(2, 3, "https://uncommitted.example")]);
    model.set_capture_fail(Some(FrameInputFailure::MergedRanges));
    orch.set_metadata_epoch(Some(1));
    orch.request_overlay_repaint();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(model.metadata_list_calls(), 4);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("committed link")
            .target()
            .as_str(),
        "https://example.com"
    );

    model.set_sheet_links(vec![link(2, 3, "https://recovered.example")]);
    model.set_capture_fail(None);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(model.metadata_list_calls(), 6);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("recovered link")
            .target()
            .as_str(),
        "https://recovered.example"
    );
}

/// A held attempt keeps the cached snapshot from the successful capture, so a
/// retry on the same epoch does not re-read the lists.
#[test]
fn a_held_attempt_retries_without_re_reading_metadata() {
    let model = model_with_metadata();
    let mut orch = build(Rc::clone(&model));
    orch.set_metadata_epoch(Some(0));
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    let reads = model.metadata_list_calls();

    // Fail a geometry read so the next attempt holds after metadata capture.
    model.set_sheet_links(vec![link(2, 3, "https://after-hold.example")]);
    model.set_row_height_bridge_fail(true);
    orch.set_metadata_epoch(Some(1));
    orch.request_repaint();
    assert_eq!(orch.render_pending(), PaintResult::RetryRequired);
    assert_eq!(model.metadata_list_calls(), reads + 2);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("committed link during hold")
            .target()
            .as_str(),
        "https://example.com"
    );

    // The retry keeps the same epoch, so the snapshot captured above is reused
    // even though the previous attempt held.
    model.set_row_height_bridge_fail(false);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);
    assert_eq!(
        orch.link_at(2, 3)
            .expect("committed link after retry")
            .target()
            .as_str(),
        "https://after-hold.example"
    );
    assert_eq!(
        model.metadata_list_calls(),
        reads + 2,
        "the retry must reuse the snapshot captured before the hold"
    );
}
