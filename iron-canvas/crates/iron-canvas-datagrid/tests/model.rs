use iron_canvas_datagrid::{Column, DataGrid};

#[test]
fn builder_sets_columns_and_rows() {
    let g = DataGrid::builder()
        .column(Column::new("Name").width(120.0))
        .column(Column::new("Qty"))
        .row(vec!["Apple".into(), "3".into()])
        .row(vec!["Pear".into(), "7".into()])
        .build();
    assert_eq!(g.column_count(), 2);
    assert_eq!(g.row_count(), 2);
    assert_eq!(g.column_width_px(0), 120.0);
    assert_eq!(g.cell_value(0, 0), Some("Apple"));
    assert_eq!(g.cell_value(1, 1), Some("7"));
}
