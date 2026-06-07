use iron_canvas_datagrid::{Column, DataGrid, SortDirection};

#[test]
fn sort_by_column_permutes_display_order_only() {
    let mut g = DataGrid::builder()
        .column(Column::new("N"))
        .row(vec!["banana".into()])
        .row(vec!["apple".into()])
        .row(vec!["cherry".into()])
        .build();
    g.sort_by(0, SortDirection::Ascending);
    assert_eq!(g.cell_value(0, 0), Some("apple"));
    assert_eq!(g.cell_value(2, 0), Some("cherry"));
    g.sort_by(0, SortDirection::Descending);
    assert_eq!(g.cell_value(0, 0), Some("cherry"));
    g.clear_sort();
    assert_eq!(g.cell_value(0, 0), Some("banana")); // insertion order restored
}
