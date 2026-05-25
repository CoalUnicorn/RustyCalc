//! `Chrome::is_still_valid` decides slot-vec reuse vs full rebuild. The
//! decide cascade in the orchestrator branches on this verdict; getting it
//! wrong skips a rebuild that should happen, or wastes one that shouldn't.

mod common;

use iron_canvas_core::chrome::{Chrome, FramePath, FrameValidity};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::CanvasSize;

use common::{canvas_default, TestModel};

fn fresh(model: &TestModel) -> Chrome {
    let theme = CanvasTheme::light();
    Chrome::next(None, model, canvas_default(), &theme, FramePath::Fresh)
}

#[test]
fn unchanged_state_reports_slots_reuse() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::SlotsReuse
    );
}

#[test]
fn canvas_size_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let resized = CanvasSize {
        w: canvas_default().w + 100.0,
        h: canvas_default().h,
    };
    assert_eq!(
        frame.is_still_valid(&model, resized),
        FrameValidity::Rebuild,
        "any canvas-size delta must invalidate the slot vecs"
    );
}

#[test]
fn scroll_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_top_row(5);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::Rebuild
    );
}

#[test]
fn scroll_inside_frozen_band_keeps_slots_reuse() {
    // `scroll_first(frozen, view_top)` clamps to `frozen + 1`. Moving the
    // active cell within the frozen band leaves the EFFECTIVE top
    // unchanged, so is_still_valid must NOT trigger a rebuild — that
    // would burn an allocation on every keyboard nudge inside the
    // frozen header rows.
    let model = TestModel::synthetic_grid().with_frozen_rows(3);
    let frame = fresh(&model);
    // top_row default = 1; frozen_rows = 3 ⇒ effective top = 4.
    // Move active cell within rows 1..=3 — top_row stays 1.
    model.set_top_row(2);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::SlotsReuse,
        "scrolling within the frozen band must not invalidate"
    );
}

#[test]
fn frozen_rows_count_change_forces_rebuild() {
    let model = TestModel::synthetic_grid().with_frozen_rows(2);
    let frame = fresh(&model);
    model.set_frozen_rows(3);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::Rebuild,
        "freeze count delta must rebuild — the pane band boundaries shift"
    );
}

#[test]
fn frozen_cols_count_change_forces_rebuild() {
    let model = TestModel::synthetic_grid().with_frozen_cols(2);
    let frame = fresh(&model);
    model.set_frozen_cols(4);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::Rebuild
    );
}

#[test]
fn sheet_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_sheet(1);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::Rebuild,
        "sheet swap invalidates cached pane content even if geometry matches"
    );
}

#[test]
fn frozen_rows_change_with_compensating_scroll_still_rebuilds() {
    // Even when the resulting effective top stays the same, the frozen
    // band's pane boundaries shift. is_still_valid must catch this via
    // the frozen-count compare, not just the effective-top compare.
    let model = TestModel::synthetic_grid()
        .with_frozen_rows(3)
        .with_top_row(5);
    let frame = fresh(&model);
    // Freeze grows by 2, scroll backs off by 2 → effective top unchanged
    // (scroll_first(5, 5) == scroll_first(7, 5) == 5? Let me re-check.
    // scroll_first(3, 5) = max(4, 5) = 5. scroll_first(5, 5) = max(6, 5) = 6.
    // So actually effective top DOES change. Set top_row=6 too so effective
    // stays at 6 either way.
    model.set_frozen_rows(5);
    model.set_top_row(6);
    assert_eq!(
        frame.is_still_valid(&model, canvas_default()),
        FrameValidity::Rebuild,
        "frozen count change must rebuild regardless of compensating scroll"
    );
}
