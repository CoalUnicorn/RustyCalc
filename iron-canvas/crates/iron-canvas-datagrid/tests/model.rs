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

#[test]
fn content_extent_sums_columns_and_rows() {
    let g = DataGrid::builder()
        .column(Column::new("").width(50.0))
        .column(Column::new("").width(70.0))
        .default_row_height(20.0)
        .row(vec!["a".into(), "b".into()])
        .row(vec!["c".into(), "d".into()])
        .row(vec!["e".into(), "f".into()])
        .build();
    assert_eq!(g.content_extent(), (120.0, 60.0));
}
