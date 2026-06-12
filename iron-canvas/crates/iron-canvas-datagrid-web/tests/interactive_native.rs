//! Stage D interactive-API proof: drives a `MemSurface` orchestrator against
//! `DataGridModel` directly (the wasm canvas constructor is unavailable in a
//! native test), mirroring each handle method's mutate + dirty + repaint
//! sequence, then inspects recorded `DrawOp`s.

use std::rc::Rc;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::Orchestrator;
use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_datagrid::{Column, DataGrid, SortDirection};
use iron_canvas_datagrid_web::DataGridModel;
use iron_canvas_recorder::{DrawOp, MemSurface};

fn fruit_grid() -> DataGrid {
    DataGrid::builder()
        .column(Column::new("Name"))
        .column(Column::new("Qty"))
        .row(vec!["Cherry".into(), "1".into()])
        .row(vec!["Apple".into(), "2".into()])
        .row(vec!["Banana".into(), "3".into()])
        .build()
}

fn new_orch(model: &Rc<DataGridModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1);
    orch.set_model(Rc::clone(model) as Rc<dyn CanvasModel>);
    orch
}

fn grid_texts(orch: &Orchestrator<MemSurface>) -> Vec<String> {
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

// x-position of the first FillText whose text matches `needle`, on the grid.
fn text_x(orch: &Orchestrator<MemSurface>, needle: &str) -> Option<f64> {
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .find_map(|op| match op {
            DrawOp::FillText { text, x, .. } if text == needle => Some(*x),
            _ => None,
        })
}

#[test]
fn scroll_changes_top_painted_row() {
    let model = Rc::new(DataGridModel::empty());
    model.replace(fruit_grid());
    let mut orch = new_orch(&model);

    // setScroll(2, 0) → 0-based JS, model is 1-based so top_row becomes 3.
    model.borrow_mut_with(|g| g.set_scroll(2 + 1, 0 + 1));
    orch.request_repaint();
    orch.paint_if_dirty();

    let texts = grid_texts(&orch);
    assert!(
        texts.iter().any(|t| t == "Banana"),
        "row 3 value must paint after scrolling to it; got {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t == "Cherry"),
        "row 1 value is above the fold and must NOT paint; got {texts:?}"
    );
}

#[test]
fn column_width_widens_following_column() {
    let model = Rc::new(DataGridModel::empty());
    model.replace(fruit_grid());

    // Baseline x of a known column-1 value.
    let mut base = new_orch(&model);
    base.request_repaint();
    base.paint_if_dirty();
    let x_before = text_x(&base, "1").expect("col-1 value must paint at default width");

    // setColumnWidth(0, 240.0): 0-based, geometry change → request_repaint.
    model.borrow_mut_with(|g| g.set_column_width(0, 240.0));
    let mut wide = new_orch(&model);
    wide.request_repaint();
    wide.paint_if_dirty();
    let x_after = text_x(&wide, "1").expect("col-1 value must still paint when col 0 widened");

    assert!(
        x_after > x_before,
        "widening column 0 must push column 1 text right: {x_before} -> {x_after}"
    );
    assert!(model.borrow_mut_with(|g| g.column_width_px(0)) >= 240.0);
}

#[test]
fn sort_orders_first_painted_data_row() {
    let model = Rc::new(DataGridModel::empty());
    model.replace(fruit_grid());
    let mut orch = new_orch(&model);

    model.borrow_mut_with(|g| g.sort_by(0, SortDirection::Ascending));
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.request_repaint();
    orch.paint_if_dirty();

    // First data value among the name column should be alphabetically first.
    let texts = grid_texts(&orch);
    let first_name = texts
        .iter()
        .find(|t| matches!(t.as_str(), "Apple" | "Banana" | "Cherry"))
        .map(String::as_str);
    assert_eq!(
        first_name,
        Some("Apple"),
        "ascending sort must paint Apple first; got {texts:?}"
    );
}

#[test]
fn idempotent_paint_records_no_new_text() {
    let model = Rc::new(DataGridModel::empty());
    model.replace(fruit_grid());
    let mut orch = new_orch(&model);

    orch.request_repaint();
    orch.paint_if_dirty();
    let count_first = grid_texts(&orch).len();

    // No change → second paint is a clean no-op (fingerprint skip).
    orch.paint_if_dirty();
    let count_second = grid_texts(&orch).len();

    assert_eq!(
        count_first, count_second,
        "a no-change paint_if_dirty must record zero new FillText ops"
    );
}

#[test]
fn select_cell_strokes_overlay() {
    let model = Rc::new(DataGridModel::empty());
    model.replace(fruit_grid());
    let mut orch = new_orch(&model);
    orch.request_repaint();
    orch.paint_if_dirty();

    // selectCell(1, 0): 0-based JS → model active+selection at 1-based (2,1).
    model.borrow_mut_with(|g| {
        g.set_active(1 + 1, 0 + 1);
        g.set_selection(1 + 1, 0 + 1, 1 + 1, 0 + 1);
    });
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    let stroked = orch
        .overlay_surface()
        .recorder()
        .ops()
        .iter()
        .any(|op| matches!(op, DrawOp::RectStroke { .. }));
    assert!(
        stroked,
        "selection move must record a stroked outline on the overlay surface"
    );
}
