//! Single merged-shift scroll-blit integration tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::chrome::{ActiveCellSnapshot, BlitOutcome, BlitPlan, Chrome, FramePath};
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{FrameDelta, FrameOutcome, GridVerdict};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn active(model: &TestModel) -> ActiveCellSnapshot {
    let view = model
        .get_selected_view()
        .expect("scroll fixture must expose a selected view");
    ActiveCellSnapshot::capture(model, view.sheet, view.row, view.column)
}

fn scroll_plan(model: &TestModel, frame: &Chrome, theme: &Rc<CanvasTheme>) -> BlitPlan {
    let inputs = test_inputs(model, canvas_default(), theme);
    let FrameDelta::Scroll(plan) =
        Chrome::classify(Some(frame), model, &inputs, Some(&active(model)))
    else {
        panic!("one-axis fixture must classify as a scroll blit");
    };
    plan
}

fn blitted_frame(
    model: &TestModel,
    previous: Chrome,
    theme: &Rc<CanvasTheme>,
    plan: &BlitPlan,
) -> Chrome {
    let inputs = test_inputs(model, canvas_default(), theme);
    let BlitOutcome::Blitted(frame) = Chrome::next_blit(Some(previous), model, &inputs, plan)
    else {
        panic!("compatible scroll must reuse Chrome in place");
    };
    frame
}

fn run_scroll(apply: impl FnOnce(&TestModel)) -> (BlitPlan, Vec<DrawOp>) {
    let model = TestModel::synthetic_grid()
        .with_data_until(60)
        .with_frozen(2, 2)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));

    apply(&model);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    let before = core.painter().ops().len();
    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    let ops = core.painter().ops()[before..].to_vec();
    (plan, ops)
}

fn assert_one_blit_before_grid(ops: &[DrawOp]) {
    let blits: Vec<_> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, DrawOp::Blit { .. }))
        .collect();
    assert_eq!(
        blits.len(),
        1,
        "one scroll must issue one merged pixel shift"
    );
    let grid = ops
        .iter()
        .position(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::Grid))
        .expect("successful blit must open the grid group");
    assert!(
        blits[0].0 < grid,
        "pixel shift must precede all repaint groups"
    );
}

#[test]
fn row_scroll_uses_one_merged_shift_before_grid_paint() {
    let (plan, ops) = run_scroll(|model| model.set_top_row(4));
    assert!(matches!(plan.axis, Axis::Row));
    assert_eq!(plan.shift.src.width, plan.shift.dst.width);
    assert_eq!(plan.shift.src.height, plan.shift.dst.height);
    assert_one_blit_before_grid(&ops);
}

#[test]
fn column_scroll_uses_one_merged_shift_before_grid_paint() {
    let (plan, ops) = run_scroll(|model| model.set_left_column(4));
    assert!(matches!(plan.axis, Axis::Column));
    assert_eq!(plan.shift.src.width, plan.shift.dst.width);
    assert_eq!(plan.shift.src.height, plan.shift.dst.height);
    assert_one_blit_before_grid(&ops);
}

#[test]
fn held_blit_moves_no_pixels_and_preserves_committed_cache() {
    let model = TestModel::synthetic_grid()
        .with_data_until(60)
        .with_frozen(2, 2)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));
    let committed_layout = core.grid_cache.layout();

    model.set_top_row(4);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    model.set_bulk_bridge_fail_from(Some(3));
    let ops = core.painter().ops().len();
    core.reset_trace();

    assert!(core.render_grid_blit(&model, &frame1, &plan));
    assert_eq!(core.painter().ops().len(), ops);
    assert_eq!(core.grid_cache.layout(), committed_layout);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Held));
    assert_eq!(core.trace().outcome, FrameOutcome::HeldOnBridgeFailure);

    model.set_bulk_bridge_fail_from(None);
    core.reset_trace();
    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Strip));
    assert_ne!(core.grid_cache.layout(), committed_layout);
}

#[test]
fn merged_shift_covers_frozen_and_scroll_cross_axis_bands() {
    let model = TestModel::synthetic_grid()
        .with_frozen(2, 2)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);

    model.set_top_row(4);
    let row_plan = scroll_plan(&model, &frame, &theme);
    assert_eq!(row_plan.shift.src.top_left.x, frame.cell_origin.x);
    assert_eq!(
        row_plan.shift.src.width,
        canvas_default().w as i32 - frame.cell_origin.x
    );

    model.set_top_row(1);
    model.set_left_column(4);
    let col_plan = scroll_plan(&model, &frame, &theme);
    assert_eq!(col_plan.shift.src.top_left.y, frame.cell_origin.y);
    assert_eq!(
        col_plan.shift.src.height,
        canvas_default().h as i32 - frame.cell_origin.y
    );
}
