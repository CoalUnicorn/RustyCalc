//! Single merged-shift scroll-blit integration tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::chrome::{
    ActiveCellSnapshot, BlitOutcome, BlitPlan, Chrome, FramePath, GridLayout, PaneRegion,
};
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::renderer::cache::BufferTruth;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{FrameDelta, FrameOutcome, GridVerdict, RCRange};
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

fn scroll_ops_with_data_until(data_until: i32, new_top: i32) -> Vec<DrawOp> {
    let model = TestModel::synthetic_grid()
        .with_data_until(data_until)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));

    model.set_top_row(new_top);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    let before = core.painter().ops().len();
    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    core.painter().ops()[before..].to_vec()
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

fn segment_range(layout: GridLayout, region: PaneRegion) -> iron_canvas_core::RCRange {
    layout
        .segments()
        .find(|segment| segment.region() == region)
        .expect("fixture layout must contain the requested segment")
        .range()
}

fn expected_widened_strip(
    previous: RCRange,
    candidate: RCRange,
    frame: &Chrome,
    region: PaneRegion,
    plan: &BlitPlan,
) -> RCRange {
    let mut expected = match plan.axis {
        Axis::Row => RCRange {
            r1: previous.r2,
            c1: candidate.c1,
            r2: candidate.r2,
            c2: candidate.c2,
        },
        Axis::Column => RCRange {
            r1: candidate.r1,
            c1: previous.c2,
            r2: candidate.r2,
            c2: candidate.c2,
        },
    };
    match plan.axis {
        Axis::Row => {
            let min = plan.pixel_strip.top_left.y;
            let max = min + plan.pixel_strip.height;
            for row in region.rows(frame) {
                if row.top + row.height > min && row.top < max {
                    expected.r1 = expected.r1.min(row.row);
                    expected.r2 = expected.r2.max(row.row);
                }
            }
        }
        Axis::Column => {
            let min = plan.pixel_strip.top_left.x;
            let max = min + plan.pixel_strip.width;
            for col in region.cols(frame) {
                if col.left + col.width > min && col.left < max {
                    expected.c1 = expected.c1.min(col.col);
                    expected.c2 = expected.c2.max(col.col);
                }
            }
        }
    }
    expected
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
fn blit_repaint_uses_the_exact_pixel_strip_clip() {
    let (plan, ops) = run_scroll(|model| model.set_top_row(4));
    let clips: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::PushClip { rect } => Some(*rect),
            _ => None,
        })
        .collect();
    assert_eq!(clips, vec![plan.pixel_strip]);
    assert_eq!(
        ops.iter()
            .filter(|op| matches!(op, DrawOp::PopClip))
            .count(),
        1
    );
}

#[test]
fn scroll_layout_transition_preserves_cache() {
    let model = TestModel::synthetic_grid()
        .with_data_until(140)
        .with_frozen(2, 2)
        .with_top_row(100)
        .with_active(100, 4);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));
    let previous = core.grid_cache.layout().expect("fresh paint seeds layout");

    model.set_top_row(101);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    let candidate = frame1.grid_layout();
    model.reset_bulk_fetch_calls();
    core.reset_trace();

    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Strip));
    assert_eq!(core.trace().blit_fallback, None);
    assert_eq!(core.grid_cache.layout(), Some(candidate));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
    assert_ne!(previous, candidate);

    let ranges = model.bulk_fetch_ranges();
    assert_eq!(ranges.len(), 2, "frozen columns require two row strips");
    for (range, region) in ranges
        .into_iter()
        .zip([PaneRegion::BottomLeft, PaneRegion::BottomRight])
    {
        let old = segment_range(previous, region);
        let new = segment_range(candidate, region);
        assert_eq!(
            range,
            expected_widened_strip(old, new, &frame1, region, &plan)
        );
        assert!(range.r1 >= 100, "deep scroll must not fetch rows 3..=99");
    }
}

#[test]
fn column_scroll_finalizes_exact_candidate_address_strips() {
    let model = TestModel::synthetic_grid()
        .with_data_until(140)
        .with_frozen_rows(2)
        .with_left_column(100)
        .with_active(4, 100);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));
    let previous = frame0.grid_layout();

    model.set_left_column(101);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    let candidate = frame1.grid_layout();
    model.reset_bulk_fetch_calls();

    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    let ranges = model.bulk_fetch_ranges();
    assert_eq!(ranges.len(), 2, "frozen rows require two column strips");
    for (range, region) in ranges
        .into_iter()
        .zip([PaneRegion::TopRight, PaneRegion::BottomRight])
    {
        let old = segment_range(previous, region);
        let new = segment_range(candidate, region);
        assert_eq!(
            range,
            expected_widened_strip(old, new, &frame1, region, &plan)
        );
        assert!(range.c1 >= 100, "deep scroll must not fetch columns 3..=99");
    }
}

#[test]
fn stale_buffers_disable_shift() {
    let model = TestModel::synthetic_grid()
        .with_data_until(60)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));

    core.grid_cache.invalidate_buffers();
    model.set_top_row(2);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    let before = core.painter().ops().len();
    model.reset_bulk_fetch_calls();
    core.reset_trace();

    assert!(!core.render_grid_blit(&model, &frame1, &plan));
    let new_ops = &core.painter().ops()[before..];
    assert!(
        !new_ops.iter().any(|op| matches!(op, DrawOp::Blit { .. })),
        "stale buffers must never be shifted"
    );
    assert_eq!(core.trace().verdict, Some(GridVerdict::Full));
    assert!(core.trace().blit_fallback.is_some());
    assert_eq!(model.bulk_fetch_calls(), 4);
    assert_eq!(
        model.bulk_fetch_ranges(),
        vec![
            frame1
                .grid_layout()
                .segments()
                .next()
                .expect("unfrozen candidate has one segment")
                .range()
        ]
    );
    assert_eq!(core.grid_cache.layout(), Some(frame1.grid_layout()));
    assert_eq!(core.grid_cache.buffer_truth(), BufferTruth::Valid);
}

#[test]
fn scroll_blit_does_not_smear_the_last_data_row_into_an_empty_strip() {
    let ops = scroll_ops_with_data_until(15, 2);
    let data_text: Vec<_> = ops
        .iter()
        .filter(|op| matches!(op, DrawOp::FillText { text, .. } if text.starts_with('R')))
        .collect();
    assert!(
        data_text.is_empty(),
        "only newly revealed empty rows may repaint: {data_text:#?}"
    );
}

#[test]
fn multirow_scroll_repaints_only_transition_data_rows() {
    let ops = scroll_ops_with_data_until(20, 6);
    let smeared: Vec<_> = ops
        .iter()
        .filter(|op| match op {
            DrawOp::FillText { text, .. } => {
                text.starts_with('R') && text != "R19" && text != "R20"
            }
            _ => false,
        })
        .collect();
    assert!(
        smeared.is_empty(),
        "kept-band data must not be repainted into the strip: {smeared:#?}"
    );
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

    core.reset_trace();
    assert!(!core.render_grid(&model, &frame1));
    assert_eq!(
        core.trace().verdict,
        Some(GridVerdict::Skip),
        "the held attempt must not corrupt history needed by the recovered row shift"
    );
}

#[test]
fn late_blit_strip_failure_rolls_back() {
    let model = TestModel::synthetic_grid()
        .with_data_until(60)
        .with_frozen_cols(2)
        .with_active(5, 5);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame0 = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame0));
    let committed = core.grid_cache.layout();

    model.set_top_row(4);
    let plan = scroll_plan(&model, &frame0, &theme);
    let frame1 = blitted_frame(&model, frame0, &theme, &plan);
    model.reset_bulk_fetch_calls();
    model.set_bulk_bridge_fail_after(Some(4));
    let ops = core.painter().ops().len();
    core.reset_trace();

    assert!(core.render_grid_blit(&model, &frame1, &plan));
    assert_eq!(core.painter().ops().len(), ops);
    assert_eq!(core.grid_cache.layout(), committed);
    assert_eq!(core.trace().verdict, Some(GridVerdict::Held));

    let capacities = core.grid_cache.preparation_scratch_capacities();
    for region in [PaneRegion::BottomLeft, PaneRegion::BottomRight] {
        let channels = capacities[region.index()];
        assert!(
            channels.0 > 0 && channels.1 > 0 && channels.2 > 0 && channels.3 > 0,
            "{region:?} strip capacity must return to the cache: {channels:?}"
        );
    }
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
