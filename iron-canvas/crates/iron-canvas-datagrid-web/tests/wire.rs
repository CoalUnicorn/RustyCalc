use iron_canvas_core::HitTest;
use iron_canvas_datagrid_web::wire::{CellWire, ColumnWire, GridDataWire, HitTestWire, RowWire};

/// A pointer on the selection's fill handle sits inside a cell rectangle, so
/// the engine hit is an `AutofillHandle`; the wire must say so. Conflating it
/// with `Cell` loses the kind before JS can decide to grab the handle, and the
/// engine coords are 1-based while every coord on the wire is 0-based.
#[test]
fn autofill_handle_hit_keeps_its_kind() {
    match HitTestWire::from(HitTest::AutofillHandle { row: 3, column: 2 }) {
        HitTestWire::AutofillHandle { row, col } => assert_eq!((row, col), (2, 1)),
        _ => panic!("autofill handle was collapsed into another hit kind"),
    }
}

fn cell(value: &str) -> CellWire {
    CellWire {
        value: value.into(),
        bold: None,
        italic: None,
        color: None,
        fill: None,
        align: None,
    }
}

#[test]
fn wire_builds_model_with_styles() {
    let wire = GridDataWire {
        columns: vec![
            ColumnWire {
                header: "Name".into(),
                width: Some(120.0),
                align: None,
            },
            ColumnWire {
                header: "Qty".into(),
                width: None,
                align: Some("right".into()),
            },
        ],
        rows: vec![
            RowWire {
                cells: vec![
                    cell("Apple"),
                    CellWire {
                        bold: Some(true),
                        ..cell("3")
                    },
                ],
                height: None,
            },
            RowWire {
                cells: vec![cell("Pear"), cell("7")],
                height: None,
            },
        ],
        row_height: Some(24.0),
    };
    let model = wire.into_model();
    assert_eq!(model.column_count(), 2);
    assert_eq!(model.cell_value(0, 0), Some("Apple"));
    assert_eq!(model.column_width_px(0), 120.0);
}
