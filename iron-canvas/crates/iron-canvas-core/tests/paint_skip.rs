//! Grid-wide fingerprint dispatch integration tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    Border, BorderItem, BorderStyle, CellStyle, FrameOutcome, GridVerdict, Line, RowSpan,
};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, canvas_large, test_inputs};

fn fixture(model: &TestModel) -> (Chrome, RendererCore<RecorderPainter>) {
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    let frame = Chrome::next(None, model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    (frame, core)
}

fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.kind = FrameKindTag::SlotsReused;
}

fn cells_group(ops: &[DrawOp]) -> &[DrawOp] {
    let start = ops
        .iter()
        .position(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::Cells))
        .expect("grid paint must open the Cells group")
        + 1;
    let end = ops[start..]
        .iter()
        .position(|op| matches!(op, DrawOp::EndGroup))
        .map(|offset| start + offset)
        .expect("grid paint must close the Cells group");
    &ops[start..end]
}

#[test]
fn grid_fallback_strokes_left_before_top() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (frame, core) = fixture(&model);

    assert!(!core.render_grid(&model, &frame));
    let ops = core.painter().ops();
    let strokes: Vec<_> = cells_group(&ops)
        .iter()
        .filter_map(|op| match op {
            DrawOp::StrokeLine { line, .. } => Some(line),
            _ => None,
        })
        .take(2)
        .collect();

    assert!(matches!(
        strokes.as_slice(),
        [Line::V { .. }, Line::H { .. }]
    ));
}

#[test]
fn explicit_borders_preserve_left_top_right_bottom_stroke_order() {
    const LEFT: &str = "#110001";
    const TOP: &str = "#220002";
    const RIGHT: &str = "#330003";
    const BOTTOM: &str = "#440004";

    fn border(color: &str) -> Option<BorderItem> {
        Some(BorderItem {
            style: BorderStyle::Thin,
            color: Some(color.to_string()),
        })
    }

    let model = TestModel::synthetic_grid().with_data_until(30);
    model.set_style(
        5,
        3,
        CellStyle {
            border: Border {
                left: border(LEFT),
                top: border(TOP),
                right: border(RIGHT),
                bottom: border(BOTTOM),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    let (frame, core) = fixture(&model);

    assert!(!core.render_grid(&model, &frame));
    let explicit_colors = [LEFT, TOP, RIGHT, BOTTOM];
    let ops = core.painter().ops();
    let strokes: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::StrokeLine { color, .. } if explicit_colors.contains(&color.as_str()) => {
                Some(color.as_str())
            }
            _ => None,
        })
        .collect();

    assert_eq!(strokes, explicit_colors);
}

#[test]
fn idempotent_grid_repaint_skips_cell_walk() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));

    promote_to_slots_reuse(&mut frame);
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
}

#[test]
fn changed_cell_selects_one_cell_envelope() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));

    promote_to_slots_reuse(&mut frame);
    model.set_cell(5, 3, "changed");
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Cell));
}

#[test]
fn cell_repaint_has_one_balanced_outer_clip() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    let range = frame
        .grid_layout()
        .segments()
        .next()
        .expect("unfrozen fixture has one segment")
        .range();
    let row = range.r1 + 2;
    model.set_cell(row, range.c1, "changed");
    let before = core.painter().ops().len();

    assert!(!core.render_grid(&model, &frame));
    let ops = core.painter().ops();
    let cell_ops = cells_group(&ops[before..]);
    assert!(
        cell_ops
            .iter()
            .any(|op| matches!(op, DrawOp::RectFill { .. }))
    );
    let pushes = cell_ops
        .iter()
        .filter(|op| matches!(op, DrawOp::PushClip { .. }))
        .count();
    let pops = cell_ops
        .iter()
        .filter(|op| matches!(op, DrawOp::PopClip))
        .count();
    assert_eq!(pushes, pops, "the outer clip and any text clips balance");
    assert!(matches!(cell_ops.first(), Some(DrawOp::PushClip { .. })));
    assert!(matches!(cell_ops.last(), Some(DrawOp::PopClip)));
}

#[test]
fn border_change_uses_the_cell_envelope() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));

    promote_to_slots_reuse(&mut frame);
    model.set_style(
        5,
        3,
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
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Cell));
}

#[test]
fn border_removal_uses_the_cell_envelope() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    let range = frame
        .grid_layout()
        .segments()
        .next()
        .expect("unfrozen fixture has one segment")
        .range();
    let row = range.r1 + 2;
    model.set_style(
        row,
        range.c1,
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
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    model.set_cell(row, range.c1, "changed");
    model.set_style(row, range.c1, CellStyle::default());
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Cell));
}

#[test]
fn more_than_eight_changed_spans_use_one_bounded_range() {
    let model = TestModel::synthetic_grid();
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_large(), &theme);
    let mut frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(Rc::new(RecorderPainter::new()));
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    let range = frame
        .grid_layout()
        .segments()
        .next()
        .expect("unfrozen fixture has one segment")
        .range();
    for delta in [0, 3, 6, 9, 12, 15, 18, 21, 24] {
        let row = range.r1 + 1 + delta;
        assert!(row < range.r2, "all changed rows must remain visible");
        model.set_cell(row, range.c1, "changed");
    }

    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Range));
}

#[test]
fn bridge_failure_holds_prior_grid_and_recovery_can_skip() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    model.set_bulk_bridge_fail(true);
    let ops_before = core.painter().ops().len();
    for _ in 0..2 {
        core.reset_trace();
        assert!(core.render_grid(&model, &frame));
        assert_eq!(core.painter().ops().len(), ops_before);
        assert_eq!(core.trace().verdict, Some(GridVerdict::Held));
        assert_eq!(core.trace().outcome, FrameOutcome::HeldOnBridgeFailure);
    }

    model.set_bulk_bridge_fail(false);
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
}

#[test]
fn damage_strip_splices_precise_history_for_next_content_check() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    model.set_cell(5, 3, "damaged");
    core.reset_trace();
    assert!(!core.render_grid_damage(&model, &frame, &[RowSpan { r1: 5, r2: 5 }]));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Strip));

    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Cell));
}

#[test]
fn frozen_layout_still_has_one_grid_verdict() {
    let model = TestModel::synthetic_grid()
        .with_data_until(30)
        .with_frozen(2, 2);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));
    promote_to_slots_reuse(&mut frame);

    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(core.trace().verdict, Some(GridVerdict::Skip));
}
