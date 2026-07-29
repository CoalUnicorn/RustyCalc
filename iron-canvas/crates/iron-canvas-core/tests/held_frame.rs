//! Stage 0/1 pins: failed paint attempts follow the transaction contract —
//! whole-frame rollback for a held viewport blit, pane-local partial commit
//! for held content panes — and always retain their work so the next tick
//! retries without any new external signal. See
//! iron-canvas/docs/designs/2026-07-27-transactional-render-pipeline.md §6-7
//! and the contract section of the Stage 0-1 plan.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::signal::RowSpan;
use iron_canvas_core::{Orchestrator, PaintRegimeTag, PaintResult};
use iron_canvas_recorder::{DrawOp, MemSurface};

use common::TestModel;

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface> {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_model(model);
    orch
}

fn grid_ops_len(orch: &Orchestrator<MemSurface>) -> usize {
    orch.grid_surface().recorder().ops().len()
}
fn overlay_ops_len(orch: &Orchestrator<MemSurface>) -> usize {
    orch.overlay_surface().recorder().ops().len()
}

fn grid_text_ops_containing(orch: &Orchestrator<MemSurface>, needle: &str) -> usize {
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::FillText { text, .. } if text.contains(needle)))
        .count()
}

/// Drive the real dispatch into the Viewport regime with a bulk-only bridge
/// failure: scroll the model, wake with OVERLAY (nav semantics — see
/// subscribe.rs), and let `decide()` discover the shift geometrically.
fn scroll_then_fail(stub: &TestModel, orch: &mut Orchestrator<MemSurface>) -> PaintResult {
    stub.set_top_row(2);
    stub.set_bulk_bridge_fail(true);
    orch.request_overlay_repaint();
    orch.paint_if_dirty()
}

#[test]
fn held_viewport_presents_nothing_and_keeps_query_geometry() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline; stamps the active-cell snapshot.
    assert_eq!(
        orch.last_trace().effective,
        Some(PaintRegimeTag::Fresh),
        "effective must be stamped on every painted frame, not only on fallback"
    );

    let rect_before = orch.cell_rect(1, 1);
    assert!(rect_before.is_some(), "row 1 visible in the baseline frame");
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    let result = scroll_then_fail(&stub, &mut orch);

    assert_eq!(
        result,
        PaintResult::Retry,
        "a held attempt must ask for a retry"
    );
    assert_eq!(grid_ops_len(&orch), grid_ops, "held: zero grid ops");
    assert_eq!(
        overlay_ops_len(&orch),
        overlay_ops,
        "held: zero overlay ops"
    );
    assert_eq!(
        orch.grid_surface().presents(),
        grid_presents,
        "held: no grid present"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        overlay_presents,
        "held: no overlay present"
    );
    // The screen still shows the un-scrolled pixels, so queries must keep
    // answering against the previous geometry (whole-frame rollback).
    assert_eq!(
        orch.cell_rect(1, 1),
        rect_before,
        "held: query geometry must stay on the last painted frame"
    );
}

#[test]
fn held_viewport_retries_after_bridge_recovery() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let result = scroll_then_fail(&stub, &mut orch);
    assert_eq!(result, PaintResult::Retry);

    stub.set_bulk_bridge_fail(false);
    let grid_ops = grid_ops_len(&orch);

    // No new external raise — the retained work must drive this paint.
    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted, "recovery paint must commit");
    assert!(
        grid_ops_len(&orch) > grid_ops,
        "recovery paint must actually repaint the grid"
    );
    assert!(
        orch.cell_rect(1, 1).is_none(),
        "after the committed scroll, row 1 is off-frame"
    );
}

#[test]
fn held_slots_reuse_retains_content_scope_and_retries() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline.

    stub.set_cell(3, 2, "edited");
    stub.set_bulk_bridge_fail(true);
    orch.mark_content_dirty(PaneRegionMask::ALL);

    let result = orch.paint_if_dirty();
    assert_eq!(result, PaintResult::Retry, "held pane must ask for a retry");
    assert_eq!(
        grid_text_ops_containing(&orch, "edited"),
        0,
        "held: the edit must not be painted from failed buffers"
    );

    stub.set_bulk_bridge_fail(false);
    let result = orch.paint_if_dirty(); // No new raise — retained work drives it.
    assert_eq!(result, PaintResult::Painted);
    assert!(
        grid_text_ops_containing(&orch, "edited") > 0,
        "recovery paint must repaint the edited cell"
    );
}

#[test]
fn held_damage_strip_retains_row_spans_and_retries() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(3, 2, "edited");
    stub.set_bulk_bridge_fail(true);
    orch.mark_rows_damaged(0, RowSpan { r1: 3, r2: 3 });

    let result = orch.paint_if_dirty();
    assert_eq!(
        result,
        PaintResult::Retry,
        "held damage strip must ask for a retry"
    );
    assert_eq!(grid_text_ops_containing(&orch, "edited"), 0);

    stub.set_bulk_bridge_fail(false);
    let result = orch.paint_if_dirty();
    assert_eq!(result, PaintResult::Painted);
    assert!(
        grid_text_ops_containing(&orch, "edited") > 0,
        "recovered damage paint must repaint the band"
    );
}

/// Review finding 2: two damage spans landing in two DIFFERENT panes (the
/// frozen-row seam lines up with the fail threshold, so TopRight gets the
/// healthy row-1 span and BottomRight gets the failing row-5 span) — the
/// cross-pane OR-fold in `render_grid_damage` must not let a later
/// successful pane overwrite an earlier pane's held verdict ("last pane
/// wins" bug). This does NOT exercise the intra-pane stop-at-first-hold
/// loop (see `render_pane_damage_stops_at_first_held_span_in_one_pane` in
/// `scroll_blit.rs` for that pin) — within either pane here, the sibling
/// span has an empty row intersection and never reaches
/// `render_pane_strip` at all.
#[test]
fn held_damage_span_in_one_pane_survives_healthy_sibling_pane() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(1, 2, "frozen-edit"); // frozen band: rows 1-2, fetches OK
    stub.set_cell(5, 2, "scroll-edit"); // scroll band: r1 >= 3, fetches fail
    stub.set_bulk_bridge_fail_from(Some(3));
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 });
    orch.mark_rows_damaged(0, RowSpan { r1: 5, r2: 5 });

    let result = orch.paint_if_dirty();
    assert_eq!(
        result,
        PaintResult::Retry,
        "one held span must mark the whole attempt Retry even when a \
         sibling span painted"
    );
    assert!(
        grid_text_ops_containing(&orch, "frozen-edit") > 0,
        "partial commit: the healthy frozen-band span paints"
    );
    assert_eq!(
        grid_text_ops_containing(&orch, "scroll-edit"),
        0,
        "the held scroll-band span paints nothing"
    );

    stub.set_bulk_bridge_fail_from(None);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(grid_text_ops_containing(&orch, "scroll-edit") > 0);
}

/// Review finding 3: pane-local partial commit, pinned on a frozen-pane
/// fixture — successful panes present and the frame advances; held panes
/// are named in the trace and retried.
#[test]
fn partial_commit_paints_healthy_panes_and_retries_held_panes() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();
    let presents_before = orch.grid_surface().presents();

    stub.set_cell(1, 3, "top-edit"); // frozen band pane — healthy
    stub.set_cell(6, 3, "bottom-edit"); // scroll band pane — fails
    stub.set_bulk_bridge_fail_from(Some(3));
    orch.mark_content_dirty(PaneRegionMask::ALL);

    let result = orch.paint_if_dirty();
    assert_eq!(result, PaintResult::Retry);
    assert!(
        orch.grid_surface().presents() > presents_before,
        "partial commit presents the painted panes"
    );
    assert!(grid_text_ops_containing(&orch, "top-edit") > 0);
    assert_eq!(grid_text_ops_containing(&orch, "bottom-edit"), 0);
    assert!(
        orch.cell_rect(1, 1).is_some(),
        "last_frame advanced under the partial-commit rule (same geometry)"
    );

    stub.set_bulk_bridge_fail_from(None);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(grid_text_ops_containing(&orch, "bottom-edit") > 0);
}

/// Scenario lifted from `blit_fallback.rs`'s
/// `blit_fallback_at_row_header_digit_boundary_returns_fresh`: scrolling from
/// top_row 980 to 981 grows the last visible row from 999 to 1000, widening
/// `row_header_thickness` past what `try_blit_reuse` allows in place. Driven
/// through the real dispatch (not the raw `Chrome` calls that test uses)
/// so `decide()` still *selects* Viewport geometrically, even though the
/// arm falls through to a full repaint. The regime and effective tags must
/// disagree here — that divergence is exactly what `effective` exists to
/// name.
#[test]
fn trace_names_selected_viewport_and_effective_fresh_on_fallback() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_top_row(980)
            .with_active(980, 1),
    );
    let mut orch = build(Rc::clone(&stub));
    // `build` sizes an 800x600 canvas; this scenario needs the row-header
    // digit-boundary geometry from `blit_fallback.rs`, which only exists at
    // 600x400. Re-resizing after construction is harmless — geometry only,
    // no model interaction.
    orch.resize(CanvasSize { w: 600.0, h: 400.0 }, 1.0);
    orch.paint_if_dirty(); // Fresh baseline at top_row=980 (last visible row 999, 3 digits).

    stub.set_top_row(981); // last visible row becomes 1000 (4 digits).
    orch.request_overlay_repaint(); // Wakes dispatch without touching CONTENT (nav semantics).
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Painted,
        "a FreshFallback still completes the frame, unlike a held Retry"
    );
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));
    let trace = orch.last_trace();
    assert_eq!(
        trace.effective,
        Some(PaintRegimeTag::Fresh),
        "a FreshFallback must be attributed as effectively Fresh"
    );
}

/// Review finding 4: a resize (Canvas2D backing-store reallocation) must
/// self-invalidate — the caller no longer follows up with `request_repaint`.
#[test]
fn resize_alone_causes_a_fresh_repaint() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();
    let grid_ops = grid_ops_len(&orch);

    orch.resize(CanvasSize { w: 900.0, h: 700.0 }, 1.0);
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Painted,
        "resize must enqueue paint work itself"
    );
    assert!(
        grid_ops_len(&orch) > grid_ops,
        "resize must repaint the grid"
    );
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
}

/// Same contract, DPR-only: the CSS size is unchanged but the backing store
/// still reallocates at the new device pixel ratio.
#[test]
fn dpr_only_resize_causes_a_fresh_repaint() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();
    let grid_ops = grid_ops_len(&orch);

    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.25); // size unchanged
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Painted,
        "a DPR change redraws at the new scale"
    );
    assert!(grid_ops_len(&orch) > grid_ops);
}

/// Composes Task 4 (resize self-invalidation) with Task 1 (viewport-hold
/// rollback): a resize arriving while a hold is outstanding must not corrupt
/// the eventual recovery, and must self-invalidate independently of the held
/// scope.
#[test]
fn resize_during_held_viewport_recovers_at_new_size() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline at 800x600, top_row=1.

    // Sanity baseline: with 20px rows and origin_y = 29 (28px header row +
    // 1px inset), an 800x600 canvas fits rows 1..=30 (`29 + 20*29 = 609 >=
    // 600`). Row 34 is off-frame here regardless of scroll, which is what
    // makes it a clean discriminator once the canvas grows.
    assert!(
        orch.cell_rect(34, 1).is_none(),
        "row 34 must be off-frame at the original 800x600 canvas"
    );

    // Hold: scroll + fail bulk fetch (same mechanism as the Task 1 viewport-hold tests).
    stub.set_top_row(2);
    stub.set_bulk_bridge_fail(true);
    orch.request_overlay_repaint();
    let result = orch.paint_if_dirty();
    assert_eq!(
        result,
        PaintResult::Retry,
        "viewport hold must ask for a retry"
    );

    // A resize arrives while the hold is outstanding — must not corrupt the
    // eventual recovery, and must self-invalidate independently of the hold.
    orch.resize(CanvasSize { w: 900.0, h: 700.0 }, 1.0);

    // Recover the bridge; the retained hold work plus the resize-raised work
    // must now dispatch together as one Fresh paint at the NEW size.
    stub.set_bulk_bridge_fail(false);
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Painted,
        "recovery after a mid-hold resize must commit"
    );
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "resize drops last_frame, forcing a full rebuild regardless of the pending hold"
    );
    // Prove the NEW size actually took effect: at top_row=2 (post-recovery),
    // a 900x700 canvas fits rows 2..=36 (`29 + 20*34 = 709 >= 700`) — row 34
    // is only reachable at the taller size, never at the original 600px one.
    assert!(
        orch.cell_rect(34, 1).is_some(),
        "row 34 must be on-frame at the new 900x700 canvas"
    );
}

/// Pins the early-return: an identical resize (same size, same DPR) must
/// not raise anything — the generic `Surface` contract does not promise an
/// identical resize is harmless, so `resize` must not even attempt one.
#[test]
fn identical_resize_is_a_no_op() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0); // same size, same dpr
    assert_eq!(
        orch.paint_if_dirty(),
        PaintResult::Idle,
        "no change, no work"
    );
}

#[test]
fn content_with_fresh_repaints_the_active_cell_overlay() {
    // Active cell stays visible after the scroll (row 5 with top_row 2).
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_active(5, 1),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();
    let overlay_before = overlay_ops_len(&orch);

    // Content edit + concurrent scroll: geometry diverges -> Fresh, with
    // CONTENT set and no OVERLAY bit raised.
    stub.set_cell(5, 1, "changed");
    stub.set_top_row(2);
    orch.mark_content_dirty(PaneRegionMask::ALL);

    let result = orch.paint_if_dirty();

    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert_eq!(result, PaintResult::Painted);
    let new_overlay: Vec<DrawOp> =
        orch.overlay_surface().recorder().ops()[overlay_before..].to_vec();
    assert!(
        new_overlay
            .iter()
            .any(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::ActiveCellRepaint)),
        "Fresh + CONTENT must run the active-cell repaint (parity with the \
         SlotsReuse and Damage arms); got {new_overlay:#?}"
    );
}

/// Pin: the only content+nav interaction in the product (Enter/Tab at the
/// viewport edge, `src/input/edit.rs:224`) reaches the engine as rowed
/// damage + un-rowed content + overlay wake, with the view already moved by
/// `scroll_into_view`. That combination must paint fresh (never blit) — the
/// stale-content rule this file's other tests already pin at the regime
/// level, proven here end-to-end for the exact real-world signal batch.
#[test]
fn commit_then_move_batch_paints_fresh_without_blit() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(40)
            .with_active(29, 1),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();
    let grid_ops = grid_ops_len(&orch);

    // The edit.rs:224 batch, as subscribe.rs routes it today:
    stub.set_cell(29, 1, "typed");
    stub.set_active(30, 1);
    stub.set_top_row(2); // scroll_into_view landed before this tick's paint
    orch.mark_rows_damaged(0, RowSpan { r1: 29, r2: 29 }); // CellChanged
    orch.mark_content_dirty(PaneRegionMask::ALL); // CalculationUpdated
    orch.request_overlay_repaint(); // SelectionChanged

    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "content + view change in one tick must not blit (stale-content rule)"
    );
    let new_ops: Vec<DrawOp> = orch.grid_surface().recorder().ops()[grid_ops..].to_vec();
    assert!(
        !new_ops.iter().any(|op| matches!(op, DrawOp::Blit { .. })),
        "no pixel shift over changed content"
    );
    assert!(
        grid_text_ops_containing(&orch, "typed") > 0,
        "the committed edit must be visible in the painted ops"
    );
}
