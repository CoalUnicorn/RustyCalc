use iron_canvas_core::CanvasModel;
use iron_canvas_datagrid::{Column, DataGrid};

#[test]
fn selection_and_scroll_reflected_in_view() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .column(Column::new("B"))
        .row(vec!["1".into(), "2".into()])
        .row(vec!["3".into(), "4".into()])
        .build();
    g.set_selection(1, 1, 2, 2);
    g.set_active(2, 1);
    g.set_scroll(2, 1);
    assert!(matches!(
        g.get_selected_view(),
        Some(v) if v.top_row == 2 && v.selection.r2 == 2 && v.row == 2
    ));
}

#[test]
fn clamp_keeps_top_row_in_bounds() {
    let mut g = DataGrid::builder()
        .column(Column::new("A"))
        .row(vec!["1".into()])
        .build();
    g.set_scroll(99, 99); // out of range
    let view = g.get_selected_view();
    assert!(matches!(view, Some(v) if v.top_row == 1 && v.left_column == 1));
}
