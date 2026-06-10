//! Stage-3 proof: the datagrid's `last_row` / `last_column` overrides reach
//! the engine through the `DataGridModel` wrapper, so scroll extents end at
//! the data instead of Excel's 1M-row bound.

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::{CanvasModel, Orchestrator};
use iron_canvas_datagrid::{Column, DataGrid};
use iron_canvas_datagrid_web::model_cell::DataGridModel;
use iron_canvas_recorder::MemSurface;

fn build(model: Rc<DataGridModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1);
    orch.set_model(model as Rc<dyn CanvasModel>);
    orch
}

#[test]
fn grid_extent_ends_at_the_data() {
    let grid = DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .row(vec!["Pear".into(), "5".into()])
        .row(vec!["Plum".into(), "7".into()])
        .build();
    let model = Rc::new(DataGridModel::empty());
    model.replace(grid);
    let mut orch = build(Rc::clone(&model));
    orch.paint_if_dirty();

    assert!(orch.cell_rect(3, 2).is_some(), "last data cell is in frame");
    assert!(
        orch.cell_rect(4, 1).is_none(),
        "row past the data must not be walked",
    );
    assert!(
        orch.cell_rect(1, 3).is_none(),
        "column past the data must not be walked",
    );
}

#[test]
fn empty_grid_keeps_one_addressable_row() {
    let model = Rc::new(DataGridModel::empty());
    let mut orch = build(Rc::clone(&model));
    orch.paint_if_dirty();

    assert!(
        orch.cell_rect(1, 1).is_some(),
        "the floor-at-1 bound keeps one empty cell addressable",
    );
    assert!(orch.cell_rect(2, 1).is_none());
}
