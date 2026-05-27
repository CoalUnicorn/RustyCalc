//! Paint contracts for the lighter decoration layers (`ClipboardLayer`,
//! `PointModeLayer`). The selection / autofill / formula-refs layers have
//! their own dedicated suites; these two were uncovered until now.
//!
//! Each layer is a pure function of (snapshot, frame, painter). Tests
//! drive the layer directly against a `RecorderPainter` (no overlay
//! orchestration) and assert the recorded `DrawOp` shape.

mod common;

use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::decoration::{ClipboardLayer, Layer, PointModeLayer};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::types::coord::{RCRange, SheetArea};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default};

fn fresh_frame(model: &TestModel) -> Chrome {
    let theme = CanvasTheme::light();
    Chrome::next(None, model, canvas_default(), &theme, FramePath::Fresh)
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
    // 600×400 canvas with 20px rows ⇒ ~19 visible rows past the header.
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
    assert!(painter.ops().is_empty(), "no point range → no paint");
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
