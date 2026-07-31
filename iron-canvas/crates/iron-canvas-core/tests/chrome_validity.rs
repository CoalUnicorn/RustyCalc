//! `Chrome::classify` decides `Stable` / `Scroll(plan)` / `Rebuild(reason)`
//! for the next frame. `plan_frame` branches on this verdict and the
//! orchestrator executes the resulting work; getting it wrong skips a rebuild
//! that should happen, wastes one that shouldn't, or picks the wrong
//! `RebuildReason` for diagnostics.
//!
//! Every `RebuildReason` variant gets its own case here, plus the `Stable`
//! and `Scroll` outcomes — this file is the classifier's spec, independent
//! of the blit-mechanics fixtures in `scroll_blit.rs` / `header_visibility.rs`
//! / `blit_fallback.rs` (which repoint the same `Chrome::classify` entry
//! point but assert on pixel/pane geometry, not the reason taxonomy).

mod common;

use std::rc::Rc;

use iron_canvas_core::CanvasSize;
use iron_canvas_core::chrome::{ActiveCellSnapshot, Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, FrameDelta, FrameInputs, RebuildReason};

use common::{TestModel, canvas_default, test_inputs};

fn light() -> Rc<CanvasTheme> {
    Rc::new(CanvasTheme::light())
}

fn fresh(model: &TestModel) -> Chrome {
    let inputs = test_inputs(model, canvas_default(), &light());
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

/// Active-cell snapshot at the model's current view — mirrors the `snap`
/// helper every scroll-blit test file defines independently.
fn snap(model: &TestModel) -> ActiveCellSnapshot {
    let view = model.get_selected_view().expect("view");
    ActiveCellSnapshot::capture(model, view.sheet, view.row, view.column)
}

#[test]
fn no_committed_frame_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(None, &model, &inputs, None);
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::NoCommittedFrame)
    ));
}

#[test]
fn unchanged_state_is_stable() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(matches!(delta, FrameDelta::Stable));
}

#[test]
fn canvas_size_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let resized = CanvasSize {
        w: canvas_default().w + 100.0,
        h: canvas_default().h,
    };
    let inputs = test_inputs(&model, resized, &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Size)),
        "any canvas-size delta must invalidate the slot vecs"
    );
}

#[test]
fn dpr_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let theme = light();
    let inputs0 = FrameInputs::capture(&model, canvas_default(), 1.0, Rc::clone(&theme), 0)
        .expect("healthy model must capture");
    let frame = Chrome::next(None, &model, &inputs0, FramePath::Fresh);
    let inputs1 = FrameInputs::capture(&model, canvas_default(), 2.0, Rc::clone(&theme), 0)
        .expect("healthy model must capture");
    let delta = Chrome::classify(Some(&frame), &model, &inputs1, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Dpr)),
        "a DPR change must invalidate the frame — Task 2 added the field \
         specifically so this comparison would be reachable"
    );
}

#[test]
fn theme_change_forces_rebuild() {
    // Altitude fix: theme identity is a frame-validity input, not an
    // out-of-band `set_theme` concern. A palette swap makes every cached
    // pixel stale, so even with identical geometry the verdict must be
    // Rebuild — else a scroll or slot reuse would repaint stale-color cells
    // under fresh chrome.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model); // built with light()
    let dark = Rc::new(CanvasTheme::dark());
    let inputs = FrameInputs::capture(&model, canvas_default(), 1.0, dark, 0)
        .expect("healthy model must capture");
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Theme)),
        "a theme change must invalidate the frame regardless of geometry"
    );
}

#[test]
fn model_generation_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let theme = light();
    let inputs0 = FrameInputs::capture(&model, canvas_default(), 1.0, Rc::clone(&theme), 0)
        .expect("healthy model must capture");
    let frame = Chrome::next(None, &model, &inputs0, FramePath::Fresh);
    let inputs1 = FrameInputs::capture(&model, canvas_default(), 1.0, theme, 1)
        .expect("healthy model must capture");
    let delta = Chrome::classify(Some(&frame), &model, &inputs1, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Model)),
        "a model_generation delta (an ordinary set_model replacement) must \
         invalidate the frame without comparing trait-object pointers"
    );
}

#[test]
fn sheet_change_forces_rebuild() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_sheet(1);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Sheet)),
        "sheet swap invalidates cached pane content even if geometry matches"
    );
}

#[test]
fn frozen_rows_count_change_forces_rebuild() {
    let model = TestModel::synthetic_grid().with_frozen_rows(2);
    let frame = fresh(&model);
    model.set_frozen_rows(3);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Freeze)),
        "freeze count delta must rebuild — the pane band boundaries shift"
    );
}

#[test]
fn frozen_cols_count_change_forces_rebuild() {
    let model = TestModel::synthetic_grid().with_frozen_cols(2);
    let frame = fresh(&model);
    model.set_frozen_cols(4);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(matches!(delta, FrameDelta::Rebuild(RebuildReason::Freeze)));
}

#[test]
fn frozen_rows_change_with_compensating_scroll_still_rebuilds() {
    // Even when the resulting effective top stays the same, the frozen
    // band's pane boundaries shift. The classifier must catch this via the
    // frozen-count compare (step 7), which runs BEFORE the scroll-origin
    // fork (step 9) — it must never fall through to Stable/Scroll just
    // because the effective top happens to match.
    let model = TestModel::synthetic_grid()
        .with_frozen_rows(3)
        .with_top_row(5);
    let frame = fresh(&model);
    // scroll_first(3, 5) = max(4, 5) = 5; scroll_first(5, 6) = max(6, 6) = 6.
    // Freeze grows by 2 and scroll advances by 1 — effective top still moves
    // (5 -> 6), so this exercises the Freeze check regardless of whether the
    // scroll-origin fork would also have fired.
    model.set_frozen_rows(5);
    model.set_top_row(6);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Rebuild(RebuildReason::Freeze)),
        "frozen count change must rebuild regardless of compensating scroll"
    );
}

#[test]
fn row_header_visibility_change_forces_rebuild() {
    let shown = TestModel::synthetic_grid();
    let hidden = TestModel::synthetic_grid().with_hidden_row_headers();
    let theme = light();
    let inputs0 = test_inputs(&shown, canvas_default(), &theme);
    let frame = Chrome::next(None, &shown, &inputs0, FramePath::Fresh);
    let inputs1 = test_inputs(&hidden, canvas_default(), &theme);
    // `Headers` fires before `model` is ever read, so passing `hidden` here
    // (rather than `shown`) is inert — see `Chrome::classify`'s doc.
    let delta = Chrome::classify(Some(&frame), &hidden, &inputs1, None);
    assert!(matches!(delta, FrameDelta::Rebuild(RebuildReason::Headers)));
}

#[test]
fn col_header_visibility_change_forces_rebuild() {
    let shown = TestModel::synthetic_grid();
    let hidden = TestModel::synthetic_grid().with_hidden_col_headers();
    let theme = light();
    let inputs0 = test_inputs(&shown, canvas_default(), &theme);
    let frame = Chrome::next(None, &shown, &inputs0, FramePath::Fresh);
    let inputs1 = test_inputs(&hidden, canvas_default(), &theme);
    let delta = Chrome::classify(Some(&frame), &hidden, &inputs1, None);
    assert!(matches!(delta, FrameDelta::Rebuild(RebuildReason::Headers)));
}

#[test]
fn scroll_inside_frozen_band_is_stable() {
    // `scroll_first(frozen, view_top)` clamps to `frozen + 1`. Moving the
    // active cell within the frozen band leaves the EFFECTIVE top
    // unchanged, so classify must report Stable — that would burn an
    // allocation on every keyboard nudge inside the frozen header rows.
    let model = TestModel::synthetic_grid().with_frozen_rows(3);
    let frame = fresh(&model);
    // top_row default = 1; frozen_rows = 3 -> effective top = 4.
    // Move active cell within rows 1..=3 — top_row stays 1.
    model.set_top_row(2);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(
        matches!(delta, FrameDelta::Stable),
        "scrolling within the frozen band must not invalidate"
    );
}

#[test]
fn two_axis_scroll_forces_rebuild() {
    // Both axes move in the same tick — no single blit shift expresses a
    // diagonal scroll. Fires before the active-cell snapshot is even
    // consulted, so `None` here still proves the point.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_top_row(2);
    model.set_left_column(2);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::TwoAxisScroll)
    ));
}

#[test]
fn single_axis_scroll_with_no_active_snapshot_forces_rebuild() {
    // A real single-axis scroll, but nothing has captured an active-cell
    // snapshot yet this attempt (e.g. pre-first-selection-refresh) — there
    // is nothing to re-hash against, so this must not be silently treated
    // as safe.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_top_row(2);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, None);
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::MissingActiveSnapshot)
    ));
}

#[test]
fn single_axis_scroll_with_changed_active_value_forces_rebuild() {
    // The canonical edit-then-scroll bug: the active cell's value changed
    // since the snapshot was captured, so the blit's kept band would carry
    // forward stale pixels for that cell.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let active_at_paint = snap(&model); // row 1's value is "" at capture time.
    model.set_data_until(5); // row 1's value flips "" -> "R1".
    model.set_top_row(2);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, Some(&active_at_paint));
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown)
    ));
}

#[test]
fn single_axis_scroll_with_unknown_active_value_forces_rebuild() {
    // A `BridgeFailed` read (at either capture or compare time) is an
    // *unknown* value, not a proof of "unchanged" — it must reject exactly
    // like a real change, never fall through to a stale-pixel blit.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    model.set_top_row(2);
    model.set_value_bridge_fail(true);
    let active = snap(&model); // captured while the bridge is down -> value_hash: None.
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, Some(&active));
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown)
    ));
}

#[test]
fn single_axis_scroll_qualifies_for_scroll_delta() {
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let active = snap(&model);
    model.set_top_row(2);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, Some(&active));
    assert!(
        matches!(delta, FrameDelta::Scroll(_)),
        "a safe single-row scroll with a matching active-cell snapshot must qualify"
    );
}

#[test]
fn incompatible_scroll_overlap_forces_rebuild() {
    // Canvas is 400px tall, rows are 20px -> ~20 visible rows. Scrolling by
    // 100 rows leaves no overlap between the previous and new viewport, so
    // the axis-specific probe has no safe kept band to reuse.
    let model = TestModel::synthetic_grid();
    let frame = fresh(&model);
    let active = snap(&model);
    model.set_top_row(101);
    let inputs = test_inputs(&model, canvas_default(), &light());
    let delta = Chrome::classify(Some(&frame), &model, &inputs, Some(&active));
    assert!(matches!(
        delta,
        FrameDelta::Rebuild(RebuildReason::IncompatibleScrollOverlap)
    ));
}
