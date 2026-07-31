//! Paint contracts for the lighter decoration layers (`ClipboardLayer`,
//! `PointModeLayer`). The selection / autofill / formula-refs layers have
//! their own dedicated suites; these two were uncovered until now.
//!
//! Each layer is a pure function of (snapshot, frame, painter). Tests
//! drive the layer directly against a `RecorderPainter` (no overlay
//! orchestration) and assert the recorded `DrawOp` shape.

mod common;

use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::decoration::{ClipboardLayer, Layer, PointModeLayer, SelectionLayer};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::types::coord::{RCRange, SheetArea};
use iron_canvas_core::{CanvasModel, FrameDelta, RebuildReason};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn fresh_frame(model: &TestModel) -> Chrome {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

// ─── ClipboardLayer ──────────────────────────────────────────────────────

#[test]
fn clipboard_empty_emits_no_ops() {
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = ClipboardLayer::default();
    layer.paint(&frame, &painter);
    assert!(
        painter.ops().is_empty(),
        "empty clipboard must not paint; got {:?}",
        painter.ops()
    );
}

#[test]
fn clipboard_wrong_sheet_emits_no_ops() {
    // Frame paints sheet 0; clipboard sits on sheet 1.
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = ClipboardLayer {
        clipboard: Some(SheetArea {
            sheet: 1,
            range: RCRange::from([2, 2, 3, 3]),
        }),
    };
    layer.paint(&frame, &painter);
    assert!(
        painter.ops().is_empty(),
        "clipboard on a different sheet must not paint"
    );
}

#[test]
fn clipboard_off_screen_emits_no_ops() {
    // 600×400 canvas with 20px rows -> ~19 visible rows past the header.
    // Row 9999 is far off-screen, so range_rect bails to None.
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = ClipboardLayer {
        clipboard: Some(SheetArea {
            sheet: 0,
            range: RCRange::from([9000, 1, 9999, 5]),
        }),
    };
    layer.paint(&frame, &painter);
    assert!(
        painter.ops().is_empty(),
        "off-screen clipboard must not paint"
    );
}

#[test]
fn clipboard_on_screen_emits_single_dashed_rect() {
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = ClipboardLayer {
        clipboard: Some(SheetArea {
            sheet: 0,
            range: RCRange::from([2, 2, 4, 4]),
        }),
    };
    layer.paint(&frame, &painter);

    let ops = painter.ops();
    assert_eq!(ops.len(), 1, "exactly one dashed rect, got {:?}", ops);
    assert!(
        matches!(ops[0], DrawOp::RectDashed { .. }),
        "marching ants must use rect_dashed, got {:?}",
        ops[0]
    );
}

// ─── PointModeLayer ──────────────────────────────────────────────────────

#[test]
fn point_mode_empty_emits_no_ops() {
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = PointModeLayer::default();
    layer.paint(&frame, &painter);
    assert!(painter.ops().is_empty(), "no point range -> no paint");
}

#[test]
fn point_mode_off_screen_emits_no_ops() {
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = PointModeLayer {
        point_range: Some(RCRange::from([9000, 1, 9999, 5])),
    };
    layer.paint(&frame, &painter);
    assert!(
        painter.ops().is_empty(),
        "off-screen point range must not paint"
    );
}

#[test]
fn point_mode_on_screen_emits_fill_then_dashed_outline() {
    // The fill must precede the dashed stroke — the 8% alpha tint would
    // otherwise wash over the dashes and mute them.
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);
    let painter = RecorderPainter::new();
    let layer = PointModeLayer {
        point_range: Some(RCRange::from([3, 3, 5, 5])),
    };
    layer.paint(&frame, &painter);

    let ops = painter.ops();
    assert_eq!(ops.len(), 2, "tint + dashed outline, got {:?}", ops);
    assert!(
        matches!(ops[0], DrawOp::RectFill { .. }),
        "fill must come first; got {:?}",
        ops[0]
    );
    assert!(
        matches!(ops[1], DrawOp::RectDashed { .. }),
        "dashed stroke must come second; got {:?}",
        ops[1]
    );
}

// ─── Stage 3 Task 2 — overlay selection and Chrome geometry share one
// captured view/sheet ──────────────────────────────────────────────────────

/// `Chrome::next` and `SelectionLayer::refresh` must be fed the SAME
/// captured `FrameInputs`, not each independently re-read the model. Proven
/// by mutating the model's sheet/active-cell/scroll AFTER capture: if either
/// consumer re-read live state, it would disagree with the other (and with
/// what capture actually saw).
#[test]
fn overlay_selection_and_chrome_geometry_use_the_same_captured_view_and_sheet() {
    let model = TestModel::synthetic_grid().with_active(3, 4);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);

    // Mutate AFTER capture. `inputs` is a frozen snapshot, so neither
    // consumer below may observe this.
    model.set_sheet(1);
    model.set_active(9, 9);
    model.set_top_row(50);

    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);

    let mut selection = SelectionLayer::default();
    selection.refresh(
        &model,
        inputs.sheet(),
        &inputs.view(),
        inputs.show_selection(),
    );

    assert_eq!(
        frame.sheet,
        inputs.sheet(),
        "Chrome geometry must reflect the captured sheet, not the model's \
         post-capture mutation to sheet 1"
    );

    let active = selection
        .active_cell
        .as_ref()
        .expect("show_selection defaults true, so refresh must capture an active cell");
    assert_eq!(
        (active.row, active.col),
        (inputs.view().row, inputs.view().column),
        "overlay selection must reflect the captured view's active cell"
    );
    assert_eq!(
        (active.row, active.col),
        (3, 4),
        "test premise: the captured active cell must be the pre-mutation (3, 4)"
    );
    assert_ne!(
        (active.row, active.col),
        (9, 9),
        "overlay selection must NOT reflect the model's post-capture mutation to (9, 9)"
    );
}

/// Stage 3 fix: `SelectionLayer::refresh` used to clear `active_cell` to
/// `None` alongside `selection_range` whenever `show_selection` was false,
/// so a data grid with selection hidden could never satisfy
/// `Chrome::classify`'s scroll-safety re-hash — every single-axis scroll
/// forced `Rebuild(MissingActiveSnapshot)` regardless of whether anything
/// about the active cell actually changed. `active_cell` must survive a
/// hidden-selection `refresh` so a selection-less host can still reach the
/// cheap `Scroll`/blit path, exactly as a selection-visible host would for
/// the identical scroll.
#[test]
fn hidden_selection_active_cell_still_qualifies_for_scroll_delta() {
    let model = TestModel::synthetic_grid();
    let frame = fresh_frame(&model);

    let view = model.get_selected_view().expect("view");
    let mut selection = SelectionLayer::default();
    selection.refresh(&model, view.sheet, &view, false); // show_selection = false

    assert!(
        selection.active_cell.is_some(),
        "active_cell must be retained as the scroll-safety snapshot even when \
         show_selection is false"
    );
    assert!(
        selection.selection_range.is_none(),
        "selection_range must still clear when show_selection is false — only \
         active_cell is retained"
    );
    assert!(
        selection.active_cell_repaint().is_none(),
        "active_cell_repaint must still suppress its paint hook when \
         show_selection is false, independent of active_cell's presence"
    );

    model.set_top_row(2); // a real, safe single-axis scroll
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let delta = Chrome::classify(
        Some(&frame),
        &model,
        &inputs,
        selection.active_cell.as_ref(),
    );

    assert!(
        !matches!(
            delta,
            FrameDelta::Rebuild(RebuildReason::MissingActiveSnapshot)
        ),
        "a hidden-selection host must not be forced to Rebuild purely because \
         selection painting is off"
    );
    assert!(
        matches!(delta, FrameDelta::Scroll(_)),
        "a safe single-row scroll with a retained active-cell snapshot must \
         qualify for Scroll even when selection is hidden"
    );
}
