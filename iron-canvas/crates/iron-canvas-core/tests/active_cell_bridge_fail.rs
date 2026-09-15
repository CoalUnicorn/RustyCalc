//! A-3 regression: the active-cell overlay repaint must be atomic with respect
//! to a transient bridge failure.
//!
//! The overlay surface is cleared at the top of every frame, so
//! `repaint_active_cell` either fully repaints the active cell or leaves the
//! grid layer's prior pixels showing through. The bug it guards against: when
//! the style fetch answers (`Value`) but a later fetch is `BridgeFailed`, the
//! old code painted an opaque background and then skipped the failed text —
//! hiding the grid's correct content behind a blank box. The fix paints
//! nothing in that case.

#![allow(clippy::unwrap_used)]

mod common;

use std::rc::Rc;

use iron_canvas_core::CellCoord;
use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::RecorderPainter;

use common::{TestModel, canvas_default, test_inputs};

fn fresh_frame(model: &TestModel) -> Chrome {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

#[test]
fn active_cell_repaints_when_every_fetch_answers() {
    // Control: with style, value, and type all answering, the active cell
    // repaints — proving the BridgeFailed test below actually exercises a
    // path that would otherwise emit ops.
    let model = TestModel::synthetic_grid().with_active(1, 1);
    let frame = fresh_frame(&model);
    let painter = Rc::new(RecorderPainter::new());
    let core = RendererCore::for_layer(Rc::clone(&painter));

    core.repaint_active_cell(&model, CellCoord { row: 1, col: 1 }, &frame);

    assert!(
        !painter.ops().is_empty(),
        "a fully-fetched active cell must repaint"
    );
}

#[test]
fn active_cell_repaint_skips_entirely_on_bridge_failure() {
    // A-3: style answers (Value, the default), but the value fetch is
    // BridgeFailed. The repaint must hold prior pixels — paint *nothing* —
    // rather than fill an opaque background over a missing text run.
    let model = TestModel::synthetic_grid().with_active(1, 1);
    model.set_value_bridge_fail(true);
    let frame = fresh_frame(&model);
    let painter = Rc::new(RecorderPainter::new());
    let core = RendererCore::for_layer(Rc::clone(&painter));

    core.repaint_active_cell(&model, CellCoord { row: 1, col: 1 }, &frame);

    assert!(
        painter.ops().is_empty(),
        "a transient bridge failure must hold prior pixels, not paint blank; got {:?}",
        painter.ops()
    );
}

#[test]
fn active_cell_coordinates_reach_the_overlay_without_transposition() {
    use iron_canvas_core::{CanvasMetrics, Orchestrator, PaintResult};
    use iron_canvas_recorder::{DrawOp, MemSurface};

    let model = Rc::new(TestModel::synthetic_grid().with_active(2, 3));
    model.set_cell(2, 3, "target cell");
    model.set_cell(3, 2, "transposed cell");
    let mut orch = Orchestrator::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasMetrics::new(canvas_default(), 1.0).expect("valid test metrics"));
    orch.set_model(model);
    assert_eq!(orch.render_pending(), PaintResult::Rendered);

    let ops = orch.overlay_surface().recorder().ops();
    let cell_text: Vec<_> = ops
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } if text.ends_with(" cell") => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(cell_text, ["target cell"]);
}
