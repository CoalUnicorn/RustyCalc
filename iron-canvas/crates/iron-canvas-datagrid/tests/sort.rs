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

/// A batch append re-sorts once, so the appended rows land in sorted display
/// order — the row indices the batch pushed into `order` must be the rows it
/// actually appended, and the display order must reflect the active sort.
#[test]
fn append_rows_batch_lands_in_sorted_display_order() {
    let mut g = DataGrid::builder()
        .column(Column::new("N"))
        .row(vec!["m".into()])
        .build();
    g.sort_by(0, SortDirection::Ascending);

    g.append_rows(vec![vec!["c".into()], vec!["a".into()], vec!["z".into()]]);

    assert_eq!(g.row_count(), 4);
    let display: Vec<&str> = (0..4)
        .map(|row| g.cell_value(row, 0).expect("every display row has a value"))
        .collect();
    assert_eq!(display, ["a", "c", "m", "z"]);
}
