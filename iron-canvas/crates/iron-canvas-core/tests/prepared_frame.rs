//! Prepared-grid rollback tests at the renderer boundary.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{FrameOutcome, GridVerdict, RowSpan};
use iron_canvas_recorder::RecorderPainter;

use common::{TestModel, canvas_default, test_inputs};

fn fresh_frame(model: &TestModel) -> Chrome {
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

#[test]
fn fresh_preflight_failure_mutates_neither_painter_nor_grid_cache() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen(2, 2);
    model.set_bulk_bridge_fail_from(Some(3));
    let frame = fresh_frame(&model);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));

    assert!(core.render_grid_fresh(&model, &frame));
    assert!(core.painter().ops().is_empty());
    assert_eq!(core.grid_cache.layout(), None);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Held));
    assert_eq!(core.trace().outcome, FrameOutcome::HeldOnBridgeFailure);
}

#[test]
fn late_fresh_segment_failure_recycles_all() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen(2, 2);
    let frame = fresh_frame(&model);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));

    model.reset_bulk_fetch_calls();
    model.set_bulk_bridge_fail_after(Some(4));
    assert!(core.render_grid_fresh(&model, &frame));

    let capacities = core.grid_cache.preparation_scratch_capacities();
    for (region, channels) in capacities.into_iter().enumerate().take(2) {
        assert!(
            channels.0 > 0 && channels.1 > 0 && channels.2 > 0 && channels.3 > 0,
            "prepared region {region} must return every fetched channel on abort: {channels:?}"
        );
    }
}

#[test]
fn slots_reuse_failure_preserves_committed_grid_cache_until_recovery() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen(2, 2);
    let mut frame = fresh_frame(&model);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame));
    frame.kind = FrameKindTag::SlotsReused;

    let layout = core.grid_cache.layout();
    let truth = core.grid_cache.buffer_truth();
    let ops = core.painter().ops().len();
    model.set_bulk_bridge_fail_from(Some(3));
    core.reset_trace();
    assert!(core.render_grid(&model, &frame));
    assert_eq!(core.painter().ops().len(), ops);
    assert_eq!(core.grid_cache.layout(), layout);
    assert_eq!(core.grid_cache.buffer_truth(), truth);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Held));

    model.set_bulk_bridge_fail_from(None);
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
}

#[test]
fn damage_failure_preserves_committed_grid_cache_and_original_pixels() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen_rows(2);
    let mut frame = fresh_frame(&model);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame));
    frame.kind = FrameKindTag::SlotsReused;

    let layout = core.grid_cache.layout();
    let truth = core.grid_cache.buffer_truth();
    let ops = core.painter().ops().len();
    model.set_cell(6, 2, "damaged");
    model.set_bulk_bridge_fail_from(Some(3));
    core.reset_trace();
    assert!(core.render_grid_damage(
        &model,
        &frame,
        &[RowSpan { r1: 1, r2: 1 }, RowSpan { r1: 6, r2: 6 }]
    ));
    assert_eq!(core.painter().ops().len(), ops);
    assert_eq!(core.grid_cache.layout(), layout);
    assert_eq!(core.grid_cache.buffer_truth(), truth);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Held));
}

#[test]
fn late_damage_strip_failure_is_atomic() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen_cols(2);
    let mut frame = fresh_frame(&model);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame));
    frame.kind = FrameKindTag::SlotsReused;

    model.reset_bulk_fetch_calls();
    model.set_bulk_bridge_fail_after(Some(4));
    let ops = core.painter().ops().len();
    assert!(core.render_grid_damage(&model, &frame, &[RowSpan { r1: 5, r2: 5 }]));
    assert_eq!(core.painter().ops().len(), ops);

    let capacities = core.strip_scratch_capacities();
    assert_eq!(capacities.len(), 2, "both prepared strips must be recycled");
    for channels in capacities {
        assert!(
            channels.0 > 0 && channels.1 > 0 && channels.2 > 0 && channels.3 > 0,
            "every recycled strip channel must retain capacity: {channels:?}"
        );
    }
}
