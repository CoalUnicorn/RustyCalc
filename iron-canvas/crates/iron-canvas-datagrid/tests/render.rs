//! Smoke test: a `DataGrid` renders header labels and cell values through the
//! real iron-canvas pipeline, recorded into a `MemSurface`. No IronCalc.

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::{Orchestrator, PaintResult};
use iron_canvas_datagrid::{Column, DataGrid};
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

fn painted(grid: DataGrid) -> Vec<String> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.set_model(Rc::new(grid));
    orch.render_pending();
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn renders_headers_and_values() {
    let g = DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .row(vec!["Pear".into(), "7".into()])
        .build();
    let t = painted(g);
    assert!(t.iter().any(|s| s == "Name"));
    assert!(t.iter().any(|s| s == "Apple"));
    assert!(t.iter().any(|s| s == "7"));
}

// Stage 3 Task 1 regression: `DataGrid::get_selected_view()` used to return
// `None` when `show_selection(false)`, overloading the same signal
// `FrameInputs::capture` treats as a transient bridge failure — which would
// hold a deliberately selection-less grid in `Retry` forever. The fix keeps
// `get_selected_view()` real and adds a separate `get_show_selection()`
// flag; this proves the grid still paints geometry, the paint commits
// (never `Retry`), and no selection fill/stroke actually draws.
#[test]
fn hidden_selection_still_paints_grid_but_draws_no_selection() {
    let g = DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .show_selection(false)
        .build();

    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.set_model(Rc::new(g));
    let result = orch.render_pending();

    assert_eq!(
        result,
        PaintResult::Rendered,
        "a selection-less grid must still commit a paint, not hold in Retry"
    );

    let grid_ops = orch.grid_surface().recorder().ops();
    assert!(
        grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::FillText { text, .. } if text == "Name")),
        "grid geometry (headers/cells) must still paint with selection hidden"
    );

    // `paint_overlay_layer` always brackets `SelectionFill`/`SelectionStroke`
    // (structural, unconditional), so the meaningful assertion is that
    // nothing draws inside them, not that the bracket is absent: with no
    // clipboard/autofill/point-mode/formula-ref/custom decoration configured
    // on this grid, a rect draw op in the overlay recording can only have
    // come from selection fill, stroke, or the autofill handle — all three
    // gated on `SelectionLayer::selection_range`, which stays `None` here.
    let overlay_ops = orch.overlay_surface().recorder().ops();
    assert!(
        overlay_ops.iter().any(|op| matches!(
            op,
            DrawOp::BeginGroup {
                class: GroupClass::SelectionFill
            }
        )),
        "the selection-fill bracket itself is still emitted (structural, unconditional)"
    );
    assert_eq!(
        overlay_ops
            .iter()
            .filter(|op| matches!(op, DrawOp::RectFill { .. } | DrawOp::RectStroke { .. }))
            .count(),
        0,
        "no selection fill/stroke/handle draw op may appear when show_selection(false)"
    );
}
