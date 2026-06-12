//! Stage-2 registry proof, CI-runnable: the hover decoration — a
//! consumer-owned custom layer — paints through the open band. Drives a
//! `MemSurface` orchestrator against `DataGridModel` directly (the wasm
//! canvas constructor is unavailable natively), mirroring `setHover`'s
//! compare-then-raise sequence, then inspects the overlay `DrawOp` log.

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::{CanvasModel, Layer, Orchestrator};
use iron_canvas_datagrid::{Column, DataGrid};
use iron_canvas_datagrid_web::DataGridModel;
use iron_canvas_datagrid_web::hover::HoverLayer;
use iron_canvas_recorder::{DrawOp, MemSurface};

fn build() -> (Orchestrator<MemSurface>, Rc<HoverLayer>) {
    let grid = DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .row(vec!["Pear".into(), "5".into()])
        .build();
    let model = Rc::new(DataGridModel::empty());
    model.replace(grid);
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1);
    orch.set_model(Rc::clone(&model) as Rc<dyn CanvasModel>);
    let hover = Rc::new(HoverLayer::default());
    orch.add_decoration(Rc::clone(&hover) as Rc<dyn Layer>);
    (orch, hover)
}

/// Ops inside the *last* `custom` bracket on the overlay surface.
fn last_custom_bracket(orch: &Orchestrator<MemSurface>) -> Vec<DrawOp> {
    let ops = orch.overlay_surface().recorder().ops();
    let Some(begin) = ops
        .iter()
        .rposition(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::Custom))
    else {
        panic!("no begin_group(custom) bracket on the overlay surface");
    };
    let Some(len) = ops[begin + 1..]
        .iter()
        .position(|op| matches!(op, DrawOp::EndGroup))
    else {
        panic!("unclosed custom bracket in the overlay log");
    };
    ops[begin + 1..begin + 1 + len].to_vec()
}

#[test]
fn hover_paints_one_stroke_through_the_custom_band() {
    let (mut orch, hover) = build();
    assert!(
        hover.set_cell(HoverLayer::cell_from_js(1, 0)),
        "first set changes state"
    );
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    let bracket = last_custom_bracket(&orch);
    assert_eq!(
        bracket.len(),
        1,
        "hover paints exactly one op; bracket was {bracket:?}",
    );
    assert!(
        matches!(bracket[0], DrawOp::RectStroke { .. }),
        "hover op must be a rect stroke; got {:?}",
        bracket[0],
    );
}

#[test]
fn hover_compare_set_dedupes_and_clear_paints_nothing() {
    let (mut orch, hover) = build();
    assert!(hover.set_cell(HoverLayer::cell_from_js(1, 0)));
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    // Pointer-move spam on the same cell: no state change, no repaint owed.
    assert!(
        !hover.set_cell(HoverLayer::cell_from_js(1, 0)),
        "same-cell set must report unchanged",
    );

    // Pointer leaves the grid (negative coords clear) — the next overlay
    // paint brackets an empty custom band.
    assert!(hover.set_cell(HoverLayer::cell_from_js(-1, -1)));
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    let bracket = last_custom_bracket(&orch);
    assert!(
        bracket.is_empty(),
        "cleared hover paints nothing; bracket was {bracket:?}",
    );
}
