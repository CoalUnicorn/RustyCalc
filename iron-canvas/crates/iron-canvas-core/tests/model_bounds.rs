//! Stage-3 spec for model-driven grid bounds: `CanvasModel::last_row` /
//! `last_column` (Excel constants by default) clamp the fresh slot walk,
//! the blit-path rebuild, and the autofill-handle guard — and the two
//! walk paths must agree at the data boundary.

mod common;

use std::rc::Rc;

use iron_canvas_core::RCRange;
use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;

use common::{TestModel, canvas_default};

fn fresh(model: &TestModel) -> Chrome {
    let theme = Rc::new(CanvasTheme::light());
    Chrome::next(None, model, canvas_default(), &theme, FramePath::Fresh)
}

#[test]
fn fresh_walk_stops_at_the_model_bound() {
    let model = TestModel::synthetic_grid()
        .with_last_row(100)
        .with_top_row(95);
    let frame = fresh(&model);
    assert!(
        frame.cell_rect(100, 1).is_some(),
        "row 100 (the bound itself) is in frame",
    );
    assert!(
        frame.cell_rect(101, 1).is_none(),
        "row 101 (past the bound) must not be walked",
    );
}

/// The defaulted methods preserve today's Excel-bound walk bit-for-bit.
#[test]
fn default_bound_preserves_excel_walk() {
    let model = TestModel::synthetic_grid().with_top_row(95);
    let frame = fresh(&model);
    assert!(frame.cell_rect(101, 1).is_some());
}

#[test]
fn autofill_handle_hides_at_the_model_last_row() {
    let model = TestModel::synthetic_grid()
        .with_last_row(100)
        .with_top_row(95);
    let frame = fresh(&model);
    assert!(
        frame
            .autofill_handle(RCRange::from([98, 1, 100, 2]))
            .is_none(),
        "selection touching the model's last row has nothing to fill into",
    );
    assert!(
        frame
            .autofill_handle(RCRange::from([97, 1, 99, 2]))
            .is_some(),
        "selection short of the bound keeps its handle",
    );
}

/// The blit-path rebuild must clamp where the fresh walk clamps —
/// otherwise a scrolled grid disagrees with its fresh paint at the
/// data boundary.
#[test]
fn blit_rebuild_agrees_with_fresh_at_the_bound() {
    let model = TestModel::synthetic_grid()
        .with_last_row(100)
        .with_top_row(90);
    let prev = fresh(&model);

    let Some(rebuilt) = prev
        .pane_set
        .rebuild_rows_for_row_scroll(&model, 95, canvas_default())
    else {
        panic!("row rebuild must qualify for a plain 90->95 scroll");
    };
    let rebuilt_ids: Vec<i32> = rebuilt.iter().map(|s| s.row).collect();

    model.set_top_row(95);
    let fresh_at_95 = fresh(&model);
    let fresh_ids: Vec<i32> = fresh_at_95
        .pane_set
        .rows
        .scroll
        .iter()
        .map(|s| s.row)
        .collect();

    assert_eq!(
        rebuilt_ids, fresh_ids,
        "blit-path rebuild and fresh walk must produce the same row band",
    );
    assert_eq!(rebuilt_ids.last(), Some(&100), "band ends at the bound");
}
