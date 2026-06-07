//! Smoke test: a `DataGrid` renders header labels and cell values through the
//! real iron-canvas pipeline, recorded into a `MemSurface`. No IronCalc.

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_datagrid::DataGrid;
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

fn painted_text(grid: DataGrid) -> Vec<String> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1);
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
fn datagrid_paints_headers_and_values_without_ironcalc() {
    let grid = DataGrid::new(
        vec!["Name".into(), "Qty".into()],
        vec![
            vec!["Apple".into(), "3".into()],
            vec!["Pear".into(), "7".into()],
        ],
    );
    let texts = painted_text(grid);

    assert!(
        texts.iter().any(|t| t == "Name"),
        "data-driven column header \"Name\" must paint; got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "Apple"),
        "cell value \"Apple\" must paint; got {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t == "7"),
        "cell value \"7\" must paint; got {texts:?}"
    );
}
