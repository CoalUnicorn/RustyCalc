//! Grid-wide fingerprint dispatch integration tests.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    Border, BorderItem, BorderStyle, CellStyle, FrameOutcome, GridVerdict, RowSpan,
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

fn rect_fills_stay_in_row(ops: &[DrawOp], top: i32, height: i32) -> bool {
    ops.iter().all(|op| match op {
        DrawOp::RectFill { rect, .. } => {
            rect.top_left.y >= top && rect.top_left.y + rect.height <= top + height
        }
        _ => true,
    })
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
fn changed_cell_repaints_only_its_grid_row() {
    let model = TestModel::synthetic_grid().with_data_until(30);
    let (mut frame, core) = fixture(&model);
    assert!(!core.render_grid(&model, &frame));

    promote_to_slots_reuse(&mut frame);
    model.set_cell(5, 3, "changed");
    core.reset_trace();
    assert!(!core.render_grid(&model, &frame));
    assert_eq!(
        core.trace().verdict,
        Some(GridVerdict::Rows { spans: 1, rows: 1 })
    );
}

#[test]
fn row_repaint_rect_fills_stay_inside_the_changed_band() {
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
    let band = frame
        .range_rect(iron_canvas_core::RCRange {
            r1: row,
            c1: range.c1,
            r2: row,
            c2: range.c2,
        })
        .expect("changed row must be visible");
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
    assert!(rect_fills_stay_in_row(
        cell_ops,
        band.top_left.y,
        band.height
    ));
}

#[test]
fn border_change_widens_grid_repaint_to_full() {
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
    assert_eq!(core.trace().verdict, Some(GridVerdict::Full));
}

#[test]
fn border_removal_widens_grid_repaint_to_full() {
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
    assert_eq!(core.trace().verdict, Some(GridVerdict::Full));
}

#[test]
fn more_than_eight_changed_spans_repaint_the_full_grid() {
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
    assert_eq!(core.trace().verdict, Some(GridVerdict::Full));
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
    assert_eq!(
        core.trace().verdict,
        Some(GridVerdict::Rows { spans: 1, rows: 1 })
    );
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
