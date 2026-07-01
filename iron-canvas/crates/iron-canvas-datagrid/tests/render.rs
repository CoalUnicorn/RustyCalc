//! Smoke test: a `DataGrid` renders header labels and cell values through the
//! real iron-canvas pipeline, recorded into a `MemSurface`. No IronCalc.

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_datagrid::{Column, DataGrid};
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

fn painted(grid: DataGrid) -> Vec<String> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.set_model(Rc::new(grid));
    orch.paint_if_dirty();
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
