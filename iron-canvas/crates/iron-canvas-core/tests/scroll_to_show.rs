//! `Orchestrator::scroll_to_show` / `legal_scroll_origin` — the viewport answers the
//! host applies before and after navigation.
//!
//! Every case here is one the model's own `window_height` arithmetic gets wrong
//! or cannot see: frozen bands, a stale `top_row` inside the frozen run, an
//! oversized cell, a collapsed pane.

#![allow(clippy::unwrap_used)]

mod common;

use std::rc::Rc;

use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::geometry::constants::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT};
use iron_canvas_core::{Orchestrator, PixelRect};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

const CANVAS: CanvasSize = CanvasSize { w: 800.0, h: 600.0 };

fn painted(model: TestModel) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CANVAS, 1.0);
    orch.set_model(Rc::new(model));
    orch.render_pending();
    orch
}

fn pane(orch: &Orchestrator<MemSurface>) -> PixelRect {
    let Some(rect) = orch.scroll_pane_rect() else {
        unreachable!("a painted frame must yield a pane rect");
    };
    rect
}

/// How many default-height rows fit in the pane in full — the count the
/// backward walk must land on.
fn rows_that_fit(orch: &Orchestrator<MemSurface>) -> i32 {
    (f64::from(pane(orch).height) / DEFAULT_ROW_HEIGHT) as i32
}

fn cols_that_fit(orch: &Orchestrator<MemSurface>) -> i32 {
    (f64::from(pane(orch).width) / DEFAULT_COL_WIDTH) as i32
}

#[test]
fn a_visible_target_asks_for_no_scroll() {
    let orch = painted(TestModel::new().with_active(2, 2));
    assert_eq!(orch.scroll_to_show(2, 2), None);
}

#[test]
fn no_painted_frame_declines() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CANVAS, 1.0);
    orch.set_model(Rc::new(TestModel::new()));
    assert_eq!(
        orch.scroll_to_show(1, 1),
        None,
        "no frame to measure against"
    );
    assert_eq!(
        orch.legal_scroll_origin(),
        Some((1, 1)),
        "the legal origin is a model question, answerable before the first paint"
    );
}

#[test]
fn a_target_above_the_pane_flushes_against_the_near_edge() {
    let orch = painted(TestModel::new().with_top_row(50));
    assert_eq!(
        orch.scroll_to_show(10, 1),
        Some((10, 1)),
        "Excel puts a backwards jump flush at the pane's top edge"
    );
}

#[test]
fn a_target_below_the_pane_scrolls_the_minimum() {
    let orch = painted(TestModel::new());
    let fits = rows_that_fit(&orch);
    let target = fits + 5;

    assert_eq!(
        orch.scroll_to_show(target, 1),
        Some((target - fits + 1, 1)),
        "the target must land on the pane's last fully visible row"
    );
}

#[test]
fn only_the_axis_that_needs_moving_moves() {
    let orch = painted(TestModel::new());
    let target_col = cols_that_fit(&orch) + 3;

    let Some((top, left)) = orch.scroll_to_show(2, target_col) else {
        unreachable!("an off-pane column must ask for a scroll");
    };
    assert_eq!(top, 1, "the row axis was already showing row 2");
    assert!(left > 1, "the column axis had to move");
}

#[test]
fn a_frozen_target_never_scrolls_its_axis() {
    let orch = painted(TestModel::new().with_frozen(3, 2).with_top_row(40));
    // Row 2 and column 1 are both inside the frozen bands — always painted, so
    // neither axis has anything to bring into view.
    assert_eq!(orch.scroll_to_show(2, 1), None);
}

#[test]
fn a_cell_taller_than_the_pane_aligns_to_the_near_edge() {
    let model = TestModel::new();
    model.set_row_height(40, 5_000.0);
    let orch = painted(model);

    assert_eq!(
        orch.scroll_to_show(40, 1),
        Some((40, 1)),
        "an unshowable cell tops out at its own leading edge, never past it"
    );
}

#[test]
fn a_collapsed_pane_scrolls_nowhere() {
    let orch = painted(TestModel::new().with_frozen(400, 400));
    assert_eq!(
        pane(&orch),
        PixelRect {
            width: 0,
            height: 0,
            ..pane(&orch)
        }
    );
    assert_eq!(orch.scroll_to_show(900, 900), None);
}

#[test]
fn a_top_row_stranded_inside_the_frozen_band_heals() {
    // Freezing does not move `top_row`, so this is the state a sheet is left in
    // the moment panes are frozen — the model says row 1, the renderer paints
    // from row 4.
    let orch = painted(TestModel::new().with_frozen(3, 0).with_top_row(1));

    assert_eq!(orch.legal_scroll_origin(), Some((4, 1)));
    assert_eq!(
        orch.scroll_to_show(5, 1),
        Some((4, 1)),
        "the answer adopts the renderer's clamp instead of the model's stale row"
    );
}

#[test]
fn hidden_rows_between_origin_and_target_do_not_consume_extent() {
    let baseline = painted(TestModel::new());
    let fits = rows_that_fit(&baseline);
    let target = fits + 5;
    assert_eq!(
        baseline.scroll_to_show(target, 1),
        Some((target - fits + 1, 1)),
        "baseline: with every row at default height the target needs a scroll"
    );

    // Collapse five rows between the origin and the target. They occupy no
    // pixels, so the same target now fits without moving the viewport at all —
    // the model's own `Σ heights > window_height` walk reaches the same answer
    // only because it also sums zero; what it cannot do is see the pane.
    let model = TestModel::new();
    for row in 2..=6 {
        model.set_row_height(row, 0.0);
    }
    assert_eq!(painted(model).scroll_to_show(target, 1), None);
}
