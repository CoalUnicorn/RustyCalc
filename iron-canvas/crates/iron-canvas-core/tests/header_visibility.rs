//! Header-visibility geometry + paint gating.

mod common;

use iron_canvas_core::CanvasSize;
use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;

use common::TestModel;

const CANVAS: CanvasSize = CanvasSize { w: 600.0, h: 400.0 };

fn frame(model: &TestModel) -> Chrome {
    Chrome::next(None, model, CANVAS, &CanvasTheme::light(), FramePath::Fresh)
}

#[test]
fn both_headers_shown_have_positive_thickness() {
    let f = frame(&TestModel::synthetic_grid());
    assert!(f.row_header_thickness > 0, "row strip present by default");
    assert!(f.col_header_thickness > 0, "col strip present by default");
    assert!(f.cell_origin.x > 0, "cells offset past the row strip");
    assert!(f.cell_origin.y > 0, "cells offset past the col strip");
}

#[test]
fn hidden_row_headers_collapse_row_thickness_and_x_origin() {
    let f = frame(&TestModel::synthetic_grid().with_hidden_row_headers());
    assert_eq!(f.row_header_thickness, 0, "row strip collapsed");
    assert_eq!(f.cell_origin.x, 0, "cells start at x=0");
    assert!(f.col_header_thickness > 0);
    assert!(f.cell_origin.y > 0);
}

#[test]
fn hidden_col_headers_collapse_col_thickness_and_y_origin() {
    let f = frame(&TestModel::synthetic_grid().with_hidden_col_headers());
    assert_eq!(f.col_header_thickness, 0, "col strip collapsed");
    assert_eq!(f.cell_origin.y, 0, "cells start at y=0");
    assert!(f.row_header_thickness > 0);
    assert!(f.cell_origin.x > 0);
}

// ── Blit frozen-band geometry: hidden-row-header regression ──────────────────
//
// When the row header is HIDDEN, `cell_origin.x == 0`. The bug in
// `try_blit_rows` reconstructed the frozen-cols band origin as
// `row_header_thickness + CELL_AREA_INSET` (== 1 when hidden) instead of
// reading `prev.cell_origin.x` (== 0). This left a stale 1-px strip at the
// left edge of the frozen-column band on every row-scroll blit.

use common::canvas_default;
use iron_canvas_core::CanvasModel;
use iron_canvas_core::chrome::{ActiveCellSnapshot, BlitPlan, PaneRegion};

fn snap_at_top(m: &TestModel) -> ActiveCellSnapshot {
    let Some(view) = m.get_selected_view() else {
        panic!("get_selected_view() returned None")
    };
    ActiveCellSnapshot::capture(m, m.get_selected_sheet(), view.row, view.column)
}

// Returns the x-origin of the BottomLeft (frozen-cols) sibling shift, or None
// if no such shift is present in the plan.
fn bottom_left_band_x(plan: &BlitPlan) -> Option<i32> {
    plan.shifts
        .iter()
        .find(|s| s.pane == PaneRegion::BottomLeft)
        .map(|s| s.src.top_left.x)
}

/// With row-headers HIDDEN and frozen cols > 0, the BottomLeft frozen-cols
/// band must start at x=0 (== cell_origin.x when hidden). The bug returned 1.
#[test]
fn frozen_cols_blit_band_starts_at_cell_origin_when_row_header_hidden() {
    let canvas = canvas_default();
    let theme = CanvasTheme::light();

    // Build a model with 2 frozen columns and hidden row headers, with enough
    // data rows that a single-row scroll has an overlap window to qualify.
    let m = TestModel::synthetic_grid()
        .with_frozen_cols(2)
        .with_hidden_row_headers();
    m.set_data_until(30);

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);

    // cell_origin.x must be 0 (confirmed by the geometry tests above).
    assert_eq!(
        frame0.cell_origin.x, 0,
        "precondition: row header hidden → cell_origin.x == 0"
    );

    m.set_top_row(2);
    let Some(plan) = frame0.screen_for_blit(&m, canvas, &theme, &snap_at_top(&m)) else {
        panic!("single-row scroll with frozen cols must qualify for blit")
    };

    let Some(band_x) = bottom_left_band_x(&plan) else {
        panic!("frozen cols > 0 must produce a BottomLeft sibling shift")
    };

    // The band must start at cell_origin.x (== 0 when header hidden).
    // Bug: returned 1 (== CELL_AREA_INSET).
    assert_eq!(
        band_x, 0,
        "frozen-cols band must start at cell_origin.x (0); got {band_x} — stale 1px strip bug"
    );
}

/// Visible-path no-op: with row-headers SHOWN and frozen cols > 0, the
/// BottomLeft band must start at cell_origin.x (> 0), same as before the fix.
#[test]
fn frozen_cols_blit_band_starts_at_cell_origin_when_row_header_shown() {
    let canvas = canvas_default();
    let theme = CanvasTheme::light();

    let m = TestModel::synthetic_grid().with_frozen_cols(2);
    m.set_data_until(30);

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);
    let expected_x = frame0.cell_origin.x;
    assert!(
        expected_x > 0,
        "precondition: row header shown → cell_origin.x > 0"
    );

    m.set_top_row(2);
    let Some(plan) = frame0.screen_for_blit(&m, canvas, &theme, &snap_at_top(&m)) else {
        panic!("single-row scroll with frozen cols must qualify for blit")
    };

    let Some(band_x) = bottom_left_band_x(&plan) else {
        panic!("frozen cols > 0 must produce a BottomLeft sibling shift")
    };

    assert_eq!(
        band_x, expected_x,
        "frozen-cols band must start at cell_origin.x ({expected_x}); got {band_x}"
    );
}

// ── Blit frozen-band geometry: hidden-col-header regression (column axis) ─────
//
// Mirror of the row-scroll regression above, for the COLUMN-scroll path. When
// the col header is HIDDEN, `cell_origin.y == 0`. The same bug in
// `try_blit_cols` reconstructed the frozen-rows band origin as
// `col_header_thickness + CELL_AREA_INSET` (== 1 when hidden) instead of
// reading `prev.cell_origin.y` (== 0), leaving a stale 1-px strip at the top
// edge of the frozen-row band on every column-scroll blit. The frozen-rows
// sibling on a column scroll is the `TopRight` pane (see `try_blit_cols`).

// Returns the y-origin of the TopRight (frozen-rows) sibling shift, or None
// if no such shift is present in the plan.
fn top_right_band_y(plan: &BlitPlan) -> Option<i32> {
    plan.shifts
        .iter()
        .find(|s| s.pane == PaneRegion::TopRight)
        .map(|s| s.src.top_left.y)
}

/// With col-headers HIDDEN and frozen rows > 0, the TopRight frozen-rows
/// band must start at y=0 (== cell_origin.y when hidden). The bug returned 1.
#[test]
fn frozen_rows_blit_band_starts_at_cell_origin_when_col_header_hidden() {
    let canvas = canvas_default();
    let theme = CanvasTheme::light();

    // Build a model with 2 frozen rows and hidden col headers, with enough
    // data that a single-column scroll has an overlap window to qualify.
    let m = TestModel::synthetic_grid()
        .with_frozen_rows(2)
        .with_hidden_col_headers();
    m.set_data_until(30);

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);

    // cell_origin.y must be 0 (confirmed by the geometry tests above).
    assert_eq!(
        frame0.cell_origin.y, 0,
        "precondition: col header hidden → cell_origin.y == 0"
    );

    m.set_left_column(2);
    let Some(plan) = frame0.screen_for_blit(&m, canvas, &theme, &snap_at_top(&m)) else {
        panic!("single-column scroll with frozen rows must qualify for blit")
    };

    let Some(band_y) = top_right_band_y(&plan) else {
        panic!("frozen rows > 0 must produce a TopRight sibling shift")
    };

    // The band must start at cell_origin.y (== 0 when header hidden).
    // Bug: returned 1 (== CELL_AREA_INSET).
    assert_eq!(
        band_y, 0,
        "frozen-rows band must start at cell_origin.y (0); got {band_y} — stale 1px strip bug"
    );
}

/// Visible-path no-op: with col-headers SHOWN and frozen rows > 0, the
/// TopRight band must start at cell_origin.y (> 0), same as before the fix.
#[test]
fn frozen_rows_blit_band_starts_at_cell_origin_when_col_header_shown() {
    let canvas = canvas_default();
    let theme = CanvasTheme::light();

    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    m.set_data_until(30);

    let frame0 = Chrome::next(None, &m, canvas, &theme, FramePath::Fresh);
    let expected_y = frame0.cell_origin.y;
    assert!(
        expected_y > 0,
        "precondition: col header shown → cell_origin.y > 0"
    );

    m.set_left_column(2);
    let Some(plan) = frame0.screen_for_blit(&m, canvas, &theme, &snap_at_top(&m)) else {
        panic!("single-column scroll with frozen rows must qualify for blit")
    };

    let Some(band_y) = top_right_band_y(&plan) else {
        panic!("frozen rows > 0 must produce a TopRight sibling shift")
    };

    assert_eq!(
        band_y, expected_y,
        "frozen-rows band must start at cell_origin.y ({expected_y}); got {band_y}"
    );
}

// ── Paint-gating tests ────────────────────────────────────────────────────────

use iron_canvas_core::Orchestrator;
use iron_canvas_core::geometry::CanvasSize as OrchCanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_recorder::{DrawOp, MemSurface};
use std::rc::Rc;

fn paint(model: Rc<TestModel>) -> Vec<DrawOp> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(OrchCanvasSize { w: 600.0, h: 400.0 }, 1);
    orch.set_model(model);
    orch.paint_if_dirty();
    orch.grid_surface().recorder().ops().clone()
}

fn has_group(ops: &[DrawOp], class: GroupClass) -> bool {
    ops.iter()
        .any(|op| matches!(op, DrawOp::BeginGroup { class: c } if *c == class))
}

fn overlay_paint(model: Rc<TestModel>) -> Vec<DrawOp> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(OrchCanvasSize { w: 600.0, h: 400.0 }, 1);
    orch.set_model(model);
    orch.paint_if_dirty();
    orch.overlay_surface().recorder().ops().clone()
}

#[test]
fn both_headers_hidden_paints_no_corner_box() {
    let shown = paint(Rc::new(TestModel::synthetic_grid()));
    let hidden = paint(Rc::new(
        TestModel::synthetic_grid()
            .with_hidden_row_headers()
            .with_hidden_col_headers(),
    ));

    assert!(
        has_group(&shown, GroupClass::Corner),
        "baseline: corner drawn"
    );
    assert!(
        !has_group(&hidden, GroupClass::Corner),
        "corner box must be gated out — otherwise its 0.5px border lines paint"
    );
    assert!(
        hidden.len() < shown.len(),
        "hiding both strips must emit strictly fewer grid ops"
    );
}

#[test]
fn hidden_headers_emit_no_selected_header_fill() {
    let theme = iron_canvas_core::CanvasTheme::light();
    let selected: &str = &theme.header_selected_bg;

    let has_sel_fill = |ops: &[DrawOp]| {
        ops.iter()
            .any(|op| matches!(op, DrawOp::RectFill { color, .. } if color.as_str() == selected))
    };

    // Default active cell (1,1) highlights its row + col header.
    let shown = overlay_paint(Rc::new(TestModel::synthetic_grid()));
    assert!(
        has_sel_fill(&shown),
        "baseline: active cell highlights its headers"
    );

    let hidden = overlay_paint(Rc::new(
        TestModel::synthetic_grid()
            .with_hidden_row_headers()
            .with_hidden_col_headers(),
    ));
    assert!(
        !has_sel_fill(&hidden),
        "hidden headers must emit no header-selection fills"
    );
}
