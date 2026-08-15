//! Grid-cache and row-fingerprint integration tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::GridVerdict;
use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::renderer::cache::BufferTruth;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::RecorderPainter;

use common::{TestModel, canvas_default, test_inputs};

fn primed(model: &TestModel) -> (Chrome, RendererCore<RecorderPainter>) {
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    let mut frame = Chrome::next(None, model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(model, &frame));
    frame.kind = FrameKindTag::SlotsReused;
    (frame, core)
}

#[test]
fn one_changed_row_across_frozen_segments_has_one_grid_rows_verdict() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen_cols(2);
    let (frame, core) = primed(&model);

    model.set_cell(5, 1, "frozen-column edit");
    model.set_cell(5, 4, "scroll-column edit");
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(
        core.trace().verdict,
        Some(GridVerdict::Rows { spans: 1, rows: 1 })
    );
}

#[test]
fn unchanged_refetch_preserves_skip() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (frame, core) = primed(&model);
    let layout = core.grid_cache.layout();

    core.grid_cache.invalidate_buffers();
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Stale);
    assert_eq!(core.grid_cache.layout(), layout);

    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
    assert_eq!(core.grid_cache.layout(), layout);
}
