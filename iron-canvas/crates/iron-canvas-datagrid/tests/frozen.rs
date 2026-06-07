use iron_canvas_core::CanvasModel;
use iron_canvas_datagrid::{Column, DataGrid};

#[test]
fn frozen_header_off_by_default_then_pins_one_row() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["1".into()])
        .build();

    assert_eq!(CanvasModel::get_frozen_rows_count(&g, 0), Some(0));

    g.set_frozen_header(true);
    assert_eq!(CanvasModel::get_frozen_rows_count(&g, 0), Some(1));
}

#[test]
fn frozen_header_builder_seeds_one_frozen_row() {
    let g = DataGrid::builder()
        .column(Column::new("A"))
        .frozen_header(true)
        .build();

    assert_eq!(CanvasModel::get_frozen_rows_count(&g, 0), Some(1));
}
