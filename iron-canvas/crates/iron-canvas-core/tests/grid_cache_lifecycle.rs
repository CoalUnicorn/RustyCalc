//! Grid-cache layout, replacement, splice, and reset lifecycle tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{ActiveCellSnapshot, BlitOutcome, Chrome, FramePath, PaneRegion};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::renderer::cache::BufferTruth;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, FrameDelta, GridVerdict, RowSpan};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn fresh_frame(model: &TestModel, theme: &Rc<CanvasTheme>) -> Chrome {
    let inputs = test_inputs(model, canvas_default(), theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

fn active(model: &TestModel) -> ActiveCellSnapshot {
    let view = model
        .get_selected_view()
        .expect("grid-cache fixture must expose a selected view");
    ActiveCellSnapshot::capture(model, view.sheet, view.row, view.column)
}

#[test]
fn unfrozen_single_region() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let theme = Rc::new(CanvasTheme::light());
    let frame = fresh_frame(&model, &theme);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));

    model.reset_bulk_fetch_calls();
    assert!(!core.render_grid_fresh(&model, &frame));

    let segments: Vec<_> = frame.grid_layout().segments().collect();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].region(), PaneRegion::BottomRight);
    assert_eq!(model.bulk_fetch_calls(), 4);
    assert_eq!(model.bulk_fetch_ranges(), vec![segments[0].range()]);
    assert_eq!(core.grid_cache.layout(), Some(frame.grid_layout()));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
}

#[test]
fn deep_scroll_fetches_only_the_four_layout_segments() {
    let model = TestModel::synthetic_grid()
        .with_data_until(140)
        .with_frozen(2, 2)
        .with_top_row(100)
        .with_left_column(100)
        .with_active(100, 100);
    let theme = Rc::new(CanvasTheme::light());
    let frame = fresh_frame(&model, &theme);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));

    model.reset_bulk_fetch_calls();
    assert!(!core.render_grid_fresh(&model, &frame));

    let expected: Vec<_> = frame
        .grid_layout()
        .segments()
        .map(|segment| segment.range())
        .collect();
    assert_eq!(expected.len(), 4);
    assert_eq!(model.bulk_fetch_ranges(), expected);
    assert_eq!(model.bulk_fetch_calls(), 16);
    for range in expected {
        assert!(range.r2 <= 2 || range.r1 >= 100);
        assert!(range.c2 <= 2 || range.c1 >= 100);
    }
}

#[test]
fn incompatible_shape_replaces_cache_atomically() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let theme = Rc::new(CanvasTheme::light());
    let frame0 = fresh_frame(&model, &theme);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid_fresh(&model, &frame0));
    let committed = frame0.grid_layout();

    model.set_frozen_rows(2);
    let frame1 = fresh_frame(&model, &theme);
    assert_ne!(committed.shape(), frame1.grid_layout().shape());
    model.reset_bulk_fetch_calls();
    model.set_bulk_bridge_fail_after(Some(4));

    assert!(core.render_grid_fresh(&model, &frame1));
    assert_eq!(core.grid_cache.layout(), Some(committed));

    model.set_bulk_bridge_fail_after(None);
    model.reset_bulk_fetch_calls();
    core.reset_trace();
    assert!(!core.render_grid_fresh(&model, &frame1));
    assert_eq!(core.grid_cache.layout(), Some(frame1.grid_layout()));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Full));
}

#[test]
fn populated_to_empty_forgets_cache() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let theme = Rc::new(CanvasTheme::light());
    let frame0 = fresh_frame(&model, &theme);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid_fresh(&model, &frame0));
    assert!(core.grid_cache.layout().is_some());

    model.set_last_row(0);
    model.set_last_column(0);
    let empty = fresh_frame(&model, &theme);
    assert_eq!(empty.grid_layout().segments().count(), 0);

    assert!(!core.render_grid_fresh(&model, &empty));
    assert_eq!(core.grid_cache.layout(), None);
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Stale);
}

#[test]
fn damage_span_crosses_freeze() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen_cols(2);
    let theme = Rc::new(CanvasTheme::light());
    let mut frame = fresh_frame(&model, &theme);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid_fresh(&model, &frame));
    frame.kind = iron_canvas_core::chrome::FrameKindTag::SlotsReused;

    model.set_cell(5, 1, "frozen-damage");
    model.set_cell(5, 4, "scroll-damage");
    model.reset_bulk_fetch_calls();
    let before = core.painter().ops().len();
    core.reset_trace();

    assert!(!core.render_grid_damage(&model, &frame, &[RowSpan { r1: 5, r2: 5 }]));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Strip));
    let ranges = model.bulk_fetch_ranges();
    assert_eq!(ranges.len(), 2);
    assert!(ranges.iter().all(|range| range.r1 == 5 && range.r2 == 5));

    let ops = core.painter().ops();
    let text: Vec<_> = ops[before..]
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains(&"frozen-damage"));
    assert!(text.contains(&"scroll-damage"));
}

#[test]
fn grid_cache_splice_shift() {
    let model = TestModel::synthetic_grid()
        .with_data_until(60)
        .with_frozen_cols(2)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let mut frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid_fresh(&model, &frame0));
    frame0.kind = iron_canvas_core::chrome::FrameKindTag::SlotsReused;

    model.set_cell(5, 1, "spliced");
    assert!(!core.render_grid_damage(&model, &frame0, &[RowSpan { r1: 5, r2: 5 }]));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);

    model.set_top_row(2);
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let FrameDelta::Scroll(plan) =
        Chrome::classify(Some(&frame0), &model, &inputs, Some(&active(&model)))
    else {
        panic!("compatible row movement must classify as a scroll");
    };
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &model, &inputs, &plan)
    else {
        panic!("compatible row movement must build a blitted frame");
    };

    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    assert_eq!(core.grid_cache.layout(), Some(frame1.grid_layout()));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
}
