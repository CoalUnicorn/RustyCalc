//! `Chrome::range_rect` and its private `range_intersects_fold` helper
//! decide whether an arbitrary sheet range can be drawn — and where.
//! Off-screen refs (`=BB3` when column BB hasn't been scrolled in) must
//! return `None`; oversized selections must clamp to canvas edges rather
//! than extending into negative pixels or off-canvas.

mod common;

use iron_canvas_core::RCRange;
use iron_canvas_core::chrome::{Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;

use common::{TestModel, canvas_default};

fn fresh(model: &TestModel) -> Chrome {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    Chrome::next(None, model, canvas_default(), &theme, FramePath::Fresh)
}

// ─── range_intersects_fold (via range_rect == None) ──────────────────────

#[test]
fn range_entirely_below_viewport_returns_none() {
    // Canvas 400 tall, 20 px rows -> ~19 visible rows. Rows 500..600 are
    // far below.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    assert!(
        frame.range_rect(RCRange::from([500, 1, 600, 5])).is_none(),
        "range past last_visible_row + past frozen_rows must not paint"
    );
}

#[test]
fn range_entirely_right_of_viewport_returns_none() {
    // Canvas 600 wide, 80 px cols -> ~7 visible cols. Cols 500..510 are
    // far right.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    assert!(
        frame.range_rect(RCRange::from([1, 500, 5, 510])).is_none(),
        "range past last_visible_col + past frozen_cols must not paint"
    );
}

#[test]
fn range_entirely_above_viewport_returns_none() {
    // Scrolled to row 100; ref points back at rows 1..3 (no frozen rows).
    let model = TestModel::synthetic_grid().with_top_row(100);
    let frame = fresh(&model);
    assert!(
        frame.range_rect(RCRange::from([1, 1, 3, 5])).is_none(),
        "range scrolled off above (range.r2 < top_row, > frozen_rows) must not paint"
    );
}

#[test]
fn range_entirely_left_of_viewport_returns_none() {
    // Scrolled right to column 50; ref points back at cols 1..3.
    let model = TestModel::synthetic_grid().with_left_column(50);
    let frame = fresh(&model);
    assert!(
        frame.range_rect(RCRange::from([1, 1, 5, 3])).is_none(),
        "range scrolled off left must not paint"
    );
}

// ─── frozen band interactions ────────────────────────────────────────────

#[test]
fn range_entirely_inside_frozen_band_paints() {
    // Frozen rows 1..=3 stay visible regardless of scroll. A range
    // entirely within the frozen band must paint even when scrolled far.
    let model = TestModel::synthetic_grid()
        .with_frozen_rows(3)
        .with_top_row(100);
    let frame = fresh(&model);
    let rect = frame
        .range_rect(RCRange::from([1, 1, 2, 3]))
        .expect("frozen-band range must paint");
    assert!(rect.width > 0 && rect.height > 0);
}

#[test]
fn range_spanning_frozen_and_scroll_bands_paints() {
    // Range crosses the frozen seam (rows 2..6 with frozen_rows=3).
    // Must paint with a rect that covers from row 2 through row 6.
    let model = TestModel::synthetic_grid().with_frozen_rows(3);
    let frame = fresh(&model);
    let rect = frame
        .range_rect(RCRange::from([2, 1, 6, 3]))
        .expect("seam-crossing range must paint");
    let p = &frame.pane_set;
    assert_eq!(
        rect.top_left.y,
        p.row_to_y(2),
        "top must anchor at row 2 inside frozen band"
    );
    // bottom edge = row 6's top + height
    assert_eq!(
        rect.top_left.y + rect.height,
        p.row_to_y(6) + p.row_extent_at(6),
        "bottom must extend through row 6 in the scroll band"
    );
}

#[test]
fn single_cell_at_frozen_seam_paints_at_scroll_band_origin() {
    // frozen_rows=2; ref is the first scrollable row (row 3 by default,
    // because top_row=1 < frozen_rows+1, scroll band starts at row 3).
    let model = TestModel::synthetic_grid().with_frozen_rows(2);
    let frame = fresh(&model);
    let rect = frame
        .range_rect(RCRange::from([3, 1, 3, 1]))
        .expect("seam cell must paint");
    let p = &frame.pane_set;
    assert_eq!(rect.top_left.y, p.row_to_y(3));
    assert_eq!(rect.height, p.row_extent_at(3));
}

// ─── oversized-range clamping ────────────────────────────────────────────

#[test]
fn range_past_last_visible_clamps_to_canvas_edges() {
    // Select rows 1..=100000 — well past the visible band. range_rect
    // should clamp the bottom to canvas height rather than computing
    // off-canvas pixel positions through row_to_y(100000) (which returns 0).
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let rect = frame
        .range_rect(RCRange::from([1, 1, 100_000, 100_000]))
        .expect("seam-crossing range must paint");
    assert_eq!(
        rect.top_left.y + rect.height,
        canvas_default().h as i32,
        "bottom edge must clamp to canvas height"
    );
    assert_eq!(
        rect.top_left.x + rect.width,
        canvas_default().w as i32,
        "right edge must clamp to canvas width"
    );
}
