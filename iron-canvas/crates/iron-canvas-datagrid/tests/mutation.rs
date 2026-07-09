use iron_canvas_datagrid::{Column, DataGrid};

#[test]
fn set_cell_and_append_rows() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .column(Column::new("B"))
        .row(vec!["x".into(), "y".into()])
        .build();
    g.set_cell(0, 1, "Y");
    assert_eq!(g.cell_value(0, 1), Some("Y"));
    g.append_row(vec!["p".into(), "q".into()]);
    assert_eq!(g.row_count(), 2);
    assert_eq!(g.cell_value(1, 0), Some("p"));
    g.set_data(vec![Column::new("Z")], vec![vec!["only".into()]]);
    assert_eq!(g.column_count(), 1);
    assert_eq!(g.row_count(), 1);
    assert_eq!(g.cell_value(0, 0), Some("only"));
}
