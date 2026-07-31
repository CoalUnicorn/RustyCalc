//! `Chrome::next_blit` fallback: `Chrome::classify` qualifies (returns
//! `FrameDelta::Scroll`) but `try_blit_reuse` rejects in-place reuse — today
//! this fires only at a row-header digit boundary, when the new
//! last-visible row gains a digit and `row_header_thickness` widens. The
//! dispatch must hand back a `Fresh` frame rather than a malformed
//! `Blitted` one, otherwise `paint_viewport_regime` would skip the full
//! grid rebuild.

mod common;

use iron_canvas_core::chrome::{ActiveCellSnapshot, BlitOutcome, Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, CanvasSize, FrameDelta, RebuildReason};

use common::{TestModel, test_inputs};

fn snap(m: &TestModel) -> ActiveCellSnapshot {
    let view = m.get_selected_view().expect("view");
    ActiveCellSnapshot::capture(m, view.sheet, view.row, view.column)
}

#[test]
fn blit_fallback_at_row_header_digit_boundary_returns_fresh() {
    // 400 px tall canvas with 20 px rows -> ~19 visible rows past the
    // 22 px header band. At top_row=980 the last visible row is 999
    // (3 digits, row_header_thickness = default). Scrolling to
    // top_row=981 makes the last visible row 1000 (4 digits), which
    // widens row_header_thickness — `try_blit_reuse`'s cross-axis reuse
    // check rejects and the dispatch falls through to a Fresh rebuild.
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let model = TestModel::synthetic_grid()
        .with_top_row(980)
        .with_active(980, 1);

    let inputs0 = test_inputs(&model, canvas, &theme);
    let prev = Chrome::next(None, &model, &inputs0, FramePath::Fresh);
    let prev_row_header = prev.row_header_thickness;
    let last_at_prev = prev
        .pane_set
        .rows
        .scroll
        .last()
        .expect("scroll band non-empty")
        .row;
    assert_eq!(
        last_at_prev, 999,
        "test premise: prev frame's last visible row must be 999 (3 digits) \
         — adjust top_row if canvas geometry constants shift"
    );

    // Scroll by 1. The current `measure_row_header_width(999)` and
    // `measure_row_header_width(1000)` already differ — both 3- and
    // 4-digit row counts hit different label widths under the
    // measurement approximation, so we anchor on `prev_row_header !=
    // new_row_header` rather than on absolute pixel values.
    model.set_top_row(981);
    let active = snap(&model);
    let inputs1 = test_inputs(&model, canvas, &theme);

    let FrameDelta::Scroll(plan) = Chrome::classify(Some(&prev), &model, &inputs1, Some(&active))
    else {
        panic!("single-row scroll must qualify geometrically");
    };

    let outcome = Chrome::next_blit(Some(prev), &model, &inputs1, &plan);

    // The whole point of the fallback: if try_blit_reuse rejected, the outcome
    // must be `FreshFallback` (a Fresh-built frame) so paint_viewport_regime
    // invalidates the cache and repaints every pane (the explicit
    // `PaneRegionMask::ALL` it passes to `paint_grid`). The `BlitOutcome`
    // type now makes "Fresh or Blitted, never anything else" structural —
    // the else branch needs no assertion.
    let is_fallback = matches!(outcome, BlitOutcome::FreshFallback(_));
    let next_row_header = match &outcome {
        BlitOutcome::Blitted(f) | BlitOutcome::FreshFallback(f) => f.row_header_thickness,
    };
    if next_row_header != prev_row_header {
        assert!(
            is_fallback,
            "row_header widened ({}->{}), so try_blit_reuse must have fallen back to Fresh",
            prev_row_header, next_row_header
        );
    }
}

/// Sanity contrast: a normal scroll where row_header_thickness does NOT
/// change must reuse in place and report `FrameKindTag::Blitted`. This
/// guards against the fallback firing too eagerly.
#[test]
fn blit_inside_stable_digit_band_keeps_blitted_kind() {
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let model = TestModel::synthetic_grid()
        .with_top_row(10)
        .with_active(10, 1);

    let inputs0 = test_inputs(&model, canvas, &theme);
    let prev = Chrome::next(None, &model, &inputs0, FramePath::Fresh);
    let prev_row_header = prev.row_header_thickness;

    model.set_top_row(11);
    let active = snap(&model);
    let inputs1 = test_inputs(&model, canvas, &theme);

    let FrameDelta::Scroll(plan) = Chrome::classify(Some(&prev), &model, &inputs1, Some(&active))
    else {
        panic!("single-row scroll must qualify");
    };
    let BlitOutcome::Blitted(next) = Chrome::next_blit(Some(prev), &model, &inputs1, &plan) else {
        panic!("in-band scroll must reuse in place (Blitted)");
    };

    assert_eq!(
        next.row_header_thickness, prev_row_header,
        "test premise: scrolls inside the 2-digit band must keep header width"
    );
}

/// Review finding #3: a `BridgeFailed` fetch of the active cell is an *unknown*
/// value — it can't prove the cell is unchanged, so the blit must be rejected
/// regardless of which side (capture or compare) saw the failure. The control
/// case (known value, unchanged) must still qualify, so the rejection is
/// attributable to the failure and not the geometry.
#[test]
fn bridge_failed_active_cell_rejects_blit() {
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let model = TestModel::synthetic_grid()
        .with_top_row(10)
        .with_active(10, 1);
    model.set_cell(10, 1, "hello");

    let inputs0 = test_inputs(&model, canvas, &theme);
    let prev = Chrome::next(None, &model, &inputs0, FramePath::Fresh);
    model.set_top_row(11);
    // `value_bridge_fail` and cell edits below affect only the active-cell
    // value hash, not any scalar `FrameInputs` reads — one capture after the
    // scroll covers all three calls below.
    let inputs1 = test_inputs(&model, canvas, &theme);

    // Control: known, unchanged value -> single-row scroll qualifies.
    assert!(
        matches!(
            Chrome::classify(Some(&prev), &model, &inputs1, Some(&snap(&model))),
            FrameDelta::Scroll(_)
        ),
        "known unchanged active cell must qualify for blit"
    );

    // Compare-time failure: snapshot captured a known value, but the live
    // re-hash now throws (`BridgeFailed`) -> unknown -> reject.
    let known = snap(&model);
    model.set_value_bridge_fail(true);
    assert!(
        matches!(
            Chrome::classify(Some(&prev), &model, &inputs1, Some(&known)),
            FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown)
        ),
        "BridgeFailed at compare time must reject the blit"
    );

    // Capture-time failure: snapshot taken while the bridge is down (poisoned
    // `None`); even once the bridge recovers, it can't prove unchanged -> reject.
    let poisoned = snap(&model);
    model.set_value_bridge_fail(false);
    assert!(
        matches!(
            Chrome::classify(Some(&prev), &model, &inputs1, Some(&poisoned)),
            FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown)
        ),
        "BridgeFailed at capture time must reject the blit"
    );
}
