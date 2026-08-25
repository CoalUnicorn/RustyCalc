//! CI-runnable render proof: drives a `MemSurface` orchestrator against
//! `DataGridModel` directly, bypassing the wasm-only canvas constructor.

use std::rc::Rc;

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_datagrid::{Column, DataGrid};
use iron_canvas_datagrid_web::DataGridModel;
use iron_canvas_recorder::{DrawOp, MemSurface};

#[test]
fn datagrid_model_paints_through_memsurface() {
    let grid = DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .build();
    let model = Rc::new(DataGridModel::empty());
    model.replace(grid);

    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 400.0, h: 300.0 }, 1.0);
    orch.set_model(Rc::clone(&model) as Rc<dyn iron_canvas_core::CanvasModel>);
    orch.render_pending();

    let texts: Vec<String> = orch
        .grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        texts.iter().any(|t| t == "Name"),
        "header must paint; got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "Apple"),
        "value must paint; got {texts:?}"
    );
}
