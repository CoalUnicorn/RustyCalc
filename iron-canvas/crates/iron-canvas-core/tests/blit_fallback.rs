//! `Chrome::next_blit` fallback: `Chrome::classify` qualifies (returns
//! `FrameDelta::Scroll`) but `try_blit_reuse` rejects in-place reuse — today
//! this fires only at a row-header digit boundary, when the new
//! last-visible row gains a digit and `row_header_thickness` widens. The
//! dispatch must hand back a `Fresh` frame rather than a malformed
//! `Blitted` one, otherwise `render_scroll_blit` would skip the full
//! grid rebuild.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{ActiveCellSnapshot, BlitOutcome, Chrome, FramePath};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    CanvasModel, CanvasSize, FrameDelta, Orchestrator, PaintResult, RebuildReason, RenderStrategy,
};
use iron_canvas_recorder::MemSurface;

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
    // must be `FreshFallback` (a Fresh-built frame) so render_scroll_blit
    // invalidates the cache and repaints the whole grid. The `BlitOutcome`
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

// ==============================================================================
// Stage 4 pin (Task 1, bullet 2): the row-header digit-boundary scenario
// above proves `Chrome::next_blit` demotes to `BlitOutcome::FreshFallback`
// when in-place reuse is rejected. Driven through the real `Orchestrator`
// dispatch (not the raw `Chrome::classify`/`next_blit` calls the rest of
// this file uses) with a bulk bridge failure added, this proves
// `render_scroll_blit`'s `FreshFallback` arm must hold atomically —
// selected Viewport, effective Fresh fallback — exactly like an ordinary
// Fresh attempt, per the Resolved Failure Policy table.
// ==============================================================================

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.set_model(model);
    orch
}

fn grid_ops_len(orch: &Orchestrator<MemSurface>) -> usize {
    orch.grid_surface().recorder().ops().len()
}

/// A `FreshFallback` uses the same whole-grid preflight as an ordinary Fresh
/// frame. Any failed segment must therefore hold before painter interaction,
/// presentation, or candidate geometry publication.
#[test]
fn held_fresh_fallback_at_row_header_digit_boundary_holds_atomically() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(980)
            .with_active(980, 1),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.render_pending(); // Fresh baseline at top_row=980 (last visible row 999, 3 digits).

    let rect_before = orch.cell_rect(980, 1);
    assert!(
        rect_before.is_some(),
        "row 980 visible in the baseline frame"
    );
    let grid_ops = grid_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();

    stub.set_top_row(981); // last visible row becomes 1000 (4 digits) -> FreshFallback.
    stub.set_bulk_bridge_fail(true);
    orch.request_overlay_repaint(); // wakes dispatch without a view/content mark (nav semantics).
    let result = orch.render_pending();

    assert_eq!(
        result,
        PaintResult::RetryRequired,
        "a FreshFallback must hold atomically on a bulk bridge failure, the \
         same as an ordinary Fresh attempt — got {result:?}"
    );
    assert_eq!(
        orch.last_strategy(),
        Some(RenderStrategy::ScrollBlit),
        "planning still selects Viewport; only the execution demotes"
    );
    assert_eq!(
        orch.last_trace().effective,
        None,
        "a held FreshFallback names no effective strategy"
    );
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "held FreshFallback: zero new grid ops"
    );
    assert_eq!(
        orch.grid_surface().presents(),
        grid_presents,
        "held FreshFallback: no grid present"
    );
    assert_eq!(
        orch.cell_rect(980, 1),
        rect_before,
        "held FreshFallback: query geometry must not advance to the failed \
         candidate's geometry"
    );
}

// ==============================================================================
// Task 3: `PreparedBlitFrame`/`BlitRollback` field coverage. The Viewport
// `held_*` fixtures in held_frame.rs (`held_viewport_presents_nothing_and_keeps_query_geometry`,
// `held_viewport_retries_after_bridge_recovery`) already prove the held-then-
// restored round trip GREEN end to end, but their model has no frozen rows
// or columns — `frozen_count` is 0 on both axes there, so
// `PreparedBlitFrame::rollback` only ever moves empty Vecs back for the
// frozen bands, and the row scroll's cross-axis Vec (`cols.scroll`) is the
// only non-trivial one exercised. The test below adds frozen columns, so a
// row scroll also exercises a non-empty `cols.frozen` — a wrong or omitted
// move in `BlitRollback`/`PaneSet::swap_scroll_axis` would show up as a
// wrong `cell_rect` for the frozen-column cell, not just the scrolled one.
// ==============================================================================

/// GREEN: proves the reversible candidate construction restores frozen AND
/// cross-axis geometry correctly on a held-then-recovered row scroll, not
/// just the scrolled band the unfrozen `held_frame.rs` fixtures cover.
#[test]
fn held_viewport_blit_with_frozen_cols_restores_and_recovers() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_frozen_cols(2)
            .with_active(5, 5),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.render_pending(); // Fresh baseline.

    // Frozen-column cell (col 1): a row scroll never shifts BottomLeft's
    // own pixels, but a rollback that dropped or corrupted
    // `pane_set.cols.frozen` would still show up here — `cell_rect` reads
    // the frozen band's slot Vec directly, independent of anything actually
    // being repainted.
    let frozen_rect_before = orch.cell_rect(3, 1);
    assert!(
        frozen_rect_before.is_some(),
        "frozen-column cell visible in baseline"
    );
    // Scroll-band cell (col 5): mirrors the existing unfrozen coverage.
    let scroll_rect_before = orch.cell_rect(3, 5);
    assert!(
        scroll_rect_before.is_some(),
        "scroll-band cell visible in baseline"
    );

    // Same wake pattern as held_frame.rs's `scroll_then_fail`: scroll the
    // model, wake with OVERLAY (nav semantics), let `decide()` discover the
    // shift geometrically.
    stub.set_top_row(2);
    stub.set_bulk_bridge_fail(true);
    orch.request_overlay_repaint();
    let result = orch.render_pending();

    assert_eq!(result, PaintResult::RetryRequired, "held blit must retry");
    assert_eq!(
        orch.cell_rect(3, 1),
        frozen_rect_before,
        "held: frozen-column geometry must roll back untouched"
    );
    assert_eq!(
        orch.cell_rect(3, 5),
        scroll_rect_before,
        "held: scroll-band geometry must roll back to the committed frame"
    );

    // Recover — no new external raise, the retained work alone drives it.
    // Column 1 sits in BottomLeft (frozen cols x *scrolled* rows): a row
    // scroll legitimately moves its Y position once committed, same as any
    // scroll-band cell — only its X position is actually frozen. Asserting
    // the whole rect unchanged here would be the wrong invariant; X alone
    // is the frozen-column property this test cares about.
    stub.set_bulk_bridge_fail(false);
    let result = orch.render_pending();
    assert_eq!(result, PaintResult::Rendered, "recovery must commit");
    let frozen_rect_after = orch.cell_rect(3, 1);
    assert!(
        frozen_rect_after.is_some(),
        "recovered frame: frozen-column cell must still be visible"
    );
    assert_eq!(
        frozen_rect_after.map(|r| r.top_left.x),
        frozen_rect_before.map(|r| r.top_left.x),
        "recovered frame: frozen-column X position is unaffected by a row scroll"
    );
}
