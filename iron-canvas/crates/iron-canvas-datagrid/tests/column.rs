use iron_canvas_core::CanvasSize;
use iron_canvas_datagrid::{Column, DataGrid, SortDirection};

#[test]
fn set_column_width_clamps_min() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["x".into()])
        .build();
    g.set_column_width(0, 200.0);
    assert_eq!(g.column_width_px(0), 200.0);
    g.set_column_width(0, 2.0); // below min
    assert!(g.column_width_px(0) >= 16.0); // clamped
    g.set_column_width(99, 50.0); // out of range: no panic, no-op
}

/// The builder path must clamp like the mutator: a `setData` payload (which
/// builds through `Column::width`) sending 0 or a negative width would
/// otherwise render an invisible column and subtract from `content_extent`.
#[test]
fn builder_clamps_column_width_to_min() {
    let g = DataGrid::builder()
        .column(Column::new("A").width(0.0))
        .column(Column::new("B").width(-40.0))
        .default_row_height(10.0)
        .row(vec!["a".into(), "b".into()])
        .build();
    assert_eq!(g.content_extent(), CanvasSize { w: 32.0, h: 10.0 });
}

#[test]
fn current_sort_reflects_state() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["b".into()])
        .row(vec!["a".into()])
        .build();
    assert_eq!(g.current_sort(), None);
    g.sort_by(0, SortDirection::Ascending);
    assert_eq!(g.current_sort(), Some((0, true)));
    g.sort_by(0, SortDirection::Descending);
    assert_eq!(g.current_sort(), Some((0, false)));
    g.clear_sort();
    assert_eq!(g.current_sort(), None);
}
