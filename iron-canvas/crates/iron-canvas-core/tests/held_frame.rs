//! Stage 0/1 pins: failed paint attempts follow the transaction contract —
//! whole-frame rollback for a held viewport blit, pane-local partial commit
//! for held content panes — and always retain their work so the next tick
//! retries without any new external signal. See
//! iron-canvas/docs/designs/2026-07-27-transactional-render-pipeline.md §6-7
//! and the contract section of the Stage 0-1 plan.

mod common;

use std::rc::Rc;

use iron_canvas_core::RowSpan;
use iron_canvas_core::chrome::{PaneRegion, PaneRegionMask};
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::{Orchestrator, PaintRegimeTag, PaintResult, PaneVerdict, WorkFlags};
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
    // The retry must still dispatch Damage: the regime requeues the original
    // sheet + row spans, not the held pane mask. Requeueing panes instead
    // would land the recovery on SlotsReuse and pay for a whole-pane walk
    // where a clipped band was already known to be sufficient.
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Damage),
        "a held Damage strip must retry as Damage, with its bands intact"
    );
    assert!(
        grid_text_ops_containing(&orch, "edited") > 0,
        "recovered damage paint must repaint the band"
    );
}

/// Whole-frame viewport hold: nothing was committed, so the ENTIRE attempt
/// is requeued — including the view and overlay marks, which never painted.
/// The pane-local partial-commit arms drop their overlay mark on retry
/// (that overlay did paint and present); the viewport arm must not, or a
/// recovered scroll would shift the grid with the selection rectangle left
/// at its pre-scroll pixel position.
#[test]
fn viewport_hold_requeues_view_and_overlay_work() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline.

    stub.set_top_row(2);
    stub.set_bulk_bridge_fail(true);
    orch.view_changed(); // view + overlay, atomically
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);

    // Recover. No new host notification — the requeued work alone drives it.
    stub.set_bulk_bridge_fail(false);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));
    assert!(
        orch.last_work_flags()
            .contains(WorkFlags::VIEW | WorkFlags::OVERLAY),
        "a whole-frame hold must requeue both marks; got {:?}",
        orch.last_work_flags()
    );
}

/// The retry requeue merges into the pending value rather than assigning to
/// it, so a producer that marks new work between the hold and the retry
/// cannot displace the retained scope — and is not displaced by it either.
/// Both edits must survive into the recovery frame.
#[test]
fn work_marked_after_a_retry_merges_with_the_retained_scope() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    // Bottom band fails, so its pane holds and is requeued.
    stub.set_cell(6, 3, "held-edit");
    stub.set_bulk_bridge_fail_from(Some(3));
    orch.mark_content_dirty(PaneRegionMask::ALL);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    assert_eq!(grid_text_ops_containing(&orch, "held-edit"), 0);

    // A fresh edit lands in the healthy frozen band before the retry tick.
    stub.set_bulk_bridge_fail_from(None);
    stub.set_cell(1, 3, "late-edit");
    orch.mark_content_dirty(PaneRegionMask::ALL);

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(
        grid_text_ops_containing(&orch, "held-edit") > 0,
        "the retained held-pane scope must survive the newly marked work"
    );
    assert!(
        grid_text_ops_containing(&orch, "late-edit") > 0,
        "the newly marked work must be serviced in the same frame"
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
/// are named in the trace and retried. The bulk-fetch-count comparison at
/// the end additionally pins that the retry narrows to *only* the held
/// (bottom) pane: both panes render "top-edit" identically whether the
/// requeue is scoped to `held` or widened to `PaneRegionMask::ALL`, so the
/// text-op assertions above cannot tell those two apart — only the fetch
/// count, taken on the recovery frame, can.
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

    stub.reset_bulk_fetch_calls();
    let result = orch.paint_if_dirty();
    assert_eq!(result, PaintResult::Retry);
    let whole_mask_calls = stub.bulk_fetch_calls();
    assert!(
        whole_mask_calls > 0,
        "sanity: the initial mask=ALL attempt must bulk-fetch both panes"
    );
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
    stub.reset_bulk_fetch_calls();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(grid_text_ops_containing(&orch, "bottom-edit") > 0);
    // The discriminating assertion: the retry must requeue a proper subset
    // of the original mask=ALL attempt (the held bottom pane only), so its
    // cache-invalidate-driven refetch costs strictly less than repeating
    // the whole mask. A requeue that regressed to `PaneRegionMask::ALL`
    // would re-invalidate the healthy top pane's cache too and cost
    // exactly `whole_mask_calls` again — this bound catches that, where
    // the grid-text/rect assertions above do not.
    assert!(
        stub.bulk_fetch_calls() < whole_mask_calls,
        "recovery must re-fetch only the held pane; got {} bulk fetch calls \
         on recovery vs {whole_mask_calls} on the original mask=ALL attempt \
         — a requeue widened back to PaneRegionMask::ALL would cost the same",
        stub.bulk_fetch_calls()
    );
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
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "a DPR-only resize self-invalidates to Fresh, same as a size change"
    );
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
    orch.view_changed(); // SelectionChanged

    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "content + view change in one tick must not blit (stale-content rule)"
    );
    assert!(
        orch.last_work_flags().contains(WorkFlags::VIEW),
        "this batch must carry the real Stage 2 VIEW mark, not just an \
         overlay wake, or Fresh here proves nothing about view dispatch; \
         got {:?}",
        orch.last_work_flags()
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

// ==============================================================================
// Stage 4 pins (Task 1): atomic hold for Fresh / first-frame Fresh, and
// all-target-panes-failed SlotsReuse/Damage.
//
// The tests below target the CURRENT Stage 3 (`d8aed9c`) code. Several are
// EXPECTED to fail — for TWO distinct reasons discovered while writing them
// (running the Fresh-path tests first, before assuming the shape of the
// bug, is what surfaced the first one):
//
// 1. `render_pane`'s bridge-failure hold branch is gated behind
//    `frame.kind.reuses_slots()` (`SlotsReused` / `Blitted` only) — see its
//    own doc comment: "A Fresh frame has no prior valid pixels to partially
//    preserve... so it always takes the unconditional full repaint". On a
//    `FramePath::Fresh` candidate (`frame.kind == Fresh`, true for the very
//    first paint AND for every later Fresh rebuild) that guard is never
//    true, so a bulk bridge failure is not merely "handled non-atomically"
//    — it is never detected at all. `render_pane` paints through with
//    whatever the bridge returned (missing cells silently render blank) and
//    reports `held = false`, i.e. success. This is a Stage 3 design choice,
//    not an oversight (a Fresh frame was assumed to have "nothing to
//    preserve"), and it is the reason `PaintResult::Retry` itself — not
//    just the ops/presents/geometry assertions after it — fails on every
//    Fresh-path test below.
// 2. `paint_slots_reuse_regime` and `paint_damage_regime` (which DO build
//    `SlotsReused`-kind frames, so `render_pane`'s hold branch fires
//    correctly there) call `self.grid.present()` unconditionally — before
//    ever inspecting the held-pane mask — and neither regime resets
//    `last_effective` on a hold, so a fully held frame still reports its
//    selected strategy as `effective`.
//
// Stage 4's prepare/commit split (Tasks 2-5) closes both gaps; each RED
// test's doc note says exactly which line of today's code produces the
// failure, so its eventual green run is not accidental.
// ==============================================================================

/// Bullet 1 (RED against d8aed9c): content+view work always selects Fresh
/// (mirrors `content_with_fresh_repaints_the_active_cell_overlay`'s setup,
/// see `plan_frame`'s "content plus view" row) and the scroll here moves row
/// 1 off-frame — so a buggy commit of the failed candidate's geometry is
/// observable as `cell_rect(1, 1)` flipping from `Some` to `None`, not just
/// as a missing paint.
///
/// Today, EVERY assertion here fails, including `PaintResult::Retry` itself:
/// `render_pane`'s bridge-hold branch never fires on a `Fresh`-kind
/// candidate (reason 1 in the section doc above), so `held` comes back
/// empty, `paint_fresh_regime` presents both layers and commits
/// `self.last_frame = Some(frame)` unconditionally, and `paint_if_dirty`
/// returns `Painted` — the pane actually paints through with the
/// `BridgeFailed` cells silently rendered blank, not held.
#[test]
fn held_fresh_content_plus_view_holds_atomically() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline at top_row=1.

    let rect_before = orch.cell_rect(1, 1);
    assert!(rect_before.is_some(), "row 1 visible in the baseline frame");
    let grid_ops = grid_ops_len(&orch);
    let overlay_ops = overlay_ops_len(&orch);
    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    stub.set_cell(5, 1, "edited");
    stub.set_top_row(5); // moves row 1 off-frame in the candidate geometry.
    stub.set_bulk_bridge_fail(true);
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.view_changed(); // content + view together always select Fresh.
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Retry,
        "a held Fresh attempt must ask for a retry"
    );
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "held Fresh: zero new grid ops"
    );
    assert_eq!(
        overlay_ops_len(&orch),
        overlay_ops,
        "held Fresh: zero new overlay ops"
    );
    assert_eq!(
        orch.grid_surface().presents(),
        grid_presents,
        "held Fresh: no grid present"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        overlay_presents,
        "held Fresh: no overlay present"
    );
    assert_eq!(
        orch.cell_rect(1, 1),
        rect_before,
        "held Fresh: query geometry must not advance to the failed candidate's geometry"
    );
}

/// Bullet 1 recovery half, kept in its own test (rather than appended to
/// `held_fresh_content_plus_view_holds_atomically`) per the project's
/// one-assertion-focus-per-test convention. RED today for the same reason as
/// its sibling above (reason 1 in the section doc): the very first
/// `assert_eq!(.., PaintResult::Retry)` already fails, since today's code
/// never holds a Fresh attempt at all. Kept as its own test anyway — once
/// Stage 4 makes the hold real, this is what proves the *recovery* half of
/// the contract independently of the hold-atomicity assertions above.
#[test]
fn held_fresh_content_plus_view_recovers_without_new_host_raise() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(5, 1, "edited");
    stub.set_top_row(5);
    stub.set_bulk_bridge_fail(true);
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);

    // No new host raise — the retained work alone must drive recovery.
    stub.set_bulk_bridge_fail(false);
    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted, "recovery must commit");
    assert!(
        orch.cell_rect(1, 1).is_none(),
        "the committed scroll must take effect once recovery lands"
    );
    assert!(grid_text_ops_containing(&orch, "edited") > 0);
}

/// Bullet 3: the very first paint attempt (no committed `Chrome` at all)
/// must hold atomically on a bulk bridge failure — no committed query
/// geometry, no present on either layer, and a plain recovery once the
/// bridge heals (see the companion recovery test below).
///
/// Stage 4's atomic Fresh path (`Orchestrator::build_and_paint_fresh` ->
/// `LayerBase::paint_grid_fresh` -> `RendererCore::prepare_fresh_panes`)
/// prepares every pane — bulk fetch and bridge-check only — before the
/// painter is touched at all, so a first-frame hold reaches neither
/// `present()` call and the grid surface gains no ops *from this attempt*.
/// The baseline is snapshotted after `build()`, not compared against an
/// absolute zero: `build`'s own `resize` call already emits a few ops onto
/// the grid surface (a DPR transform plus a paint-cache invalidation) as
/// part of allocating the backing store — legitimate initialization, not a
/// paint attempt, and explicitly outside what Stage 4's atomicity contract
/// covers (see the Stage 4 brief's own `rg` audit note: "initialization,
/// resize/model-reset, and accessor code may still contain separate state
/// writes where they are not completing a paint attempt"). Every sibling
/// held-* test in this file already measures the same way; this one is the
/// only one where "before" has to mean "right after `build()`" instead of
/// "after an earlier successful paint".
#[test]
fn held_first_frame_fresh_holds_atomically() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    stub.set_bulk_bridge_fail(true);
    // No `paint_if_dirty` call before this one — `build` only queues
    // geometry + panes(ALL) + overlay work via `resize`/`set_model`, so this
    // is genuinely the first paint attempt.
    let mut orch = build(Rc::clone(&stub));
    let grid_ops = grid_ops_len(&orch);

    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Retry,
        "a held first frame must retry, not silently stay Idle"
    );
    assert!(
        orch.cell_rect(1, 1).is_none(),
        "no committed Chrome/query geometry before the first successful paint"
    );
    assert_eq!(
        orch.grid_surface().presents(),
        0,
        "a held first frame must not present the grid"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        0,
        "a held first frame must not present the overlay"
    );
    assert_eq!(
        grid_ops_len(&orch),
        grid_ops,
        "a held first frame must emit no grid ops of its own"
    );
}

/// RED today for the same reason as `held_first_frame_fresh_holds_atomically`
/// (its own `assert_eq!(.., PaintResult::Retry)` already fails) — kept as
/// its own test so it independently pins the recovery contract once Stage 4
/// makes the hold above real.
#[test]
fn held_first_frame_fresh_recovers_without_new_host_raise() {
    let stub = Rc::new(TestModel::synthetic_grid().with_data_until(30));
    stub.set_bulk_bridge_fail(true);
    let mut orch = build(Rc::clone(&stub));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);

    stub.set_bulk_bridge_fail(false);
    let result = orch.paint_if_dirty();

    assert_eq!(
        result,
        PaintResult::Painted,
        "recovery must commit normally"
    );
    assert!(orch.cell_rect(1, 1).is_some());
}

/// Bullet 6 (RED against d8aed9c): when EVERY targeted pane fails (not just
/// some), the attempt is a true hold — `Held`, not `Partial` — and must
/// present neither layer. Uses a frozen-row fixture so two real panes
/// (TopRight + BottomRight) are in scope, and the unconditional
/// `set_bulk_bridge_fail(true)` (not the row-scoped `_from` variant) fails
/// both of them, not just one.
///
/// Today, `paint_slots_reuse_regime` calls `self.grid.present()`
/// unconditionally (before checking `held`) and paints+presents the overlay
/// whenever `OverlayWork::Paint` was planned, regardless of whether every
/// pane held — so both counters advance even though nothing committed.
/// `last_effective` is also never reset on a hold, so it keeps naming
/// `SlotsReuse` instead of `None`.
#[test]
fn held_slots_reuse_all_panes_failed_is_held_not_partial() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline: primes TopRight + BottomRight.

    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    stub.set_cell(1, 1, "top-edit");
    stub.set_cell(6, 1, "bottom-edit");
    stub.set_bulk_bridge_fail(true); // unconditional: every pane's fetch fails.
    orch.mark_content_dirty(PaneRegionMask::ALL);

    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Retry);
    assert_eq!(
        orch.grid_surface().presents(),
        grid_presents,
        "an all-failed attempt is Held, not Partial: it must not present the grid"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        overlay_presents,
        "an all-failed attempt must not present the overlay either"
    );
    assert_eq!(
        orch.last_trace().effective,
        None,
        "a fully held attempt names no effective strategy"
    );
}

/// Fix-round regression (post Task 5 review): a full SlotsReuse hold must
/// retry the COMPLETE consumed work, not a pane-mask reconstruction — the
/// Resolved Failure Policy's "every target pane fails -> Held -> complete
/// consumed work" row is explicit, and `retry_for_held_panes(mask)` only
/// ever rebuilds the `content` field, silently dropping any `overlay` (or
/// `view`) bit that rode along on the same `PendingWork`. A held attempt
/// never runs the overlay refresh itself (see `finish_attempt`'s doc), so a
/// dropped mark has no other chance to be serviced.
///
/// `with_show_selection(false)` is load-bearing, not incidental fixture
/// noise: `plan_frame`'s `content_overlay = work.has_overlay() ||
/// show_selection` would otherwise select `OverlayWork::Paint` on the
/// recovery attempt from `show_selection` alone, regardless of whether the
/// `overlay` bit itself actually survived the retry — masking the exact
/// bug this test exists to catch. With `show_selection` held at `false`
/// throughout, an overlay repaint on the recovery frame can only mean the
/// mark was genuinely retried.
#[test]
fn held_slots_reuse_full_hold_retries_the_complete_consumed_work() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_show_selection(false),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline.

    stub.set_cell(1, 1, "edited");
    stub.set_bulk_bridge_fail(true); // unconditional: every pane's fetch fails.
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.request_overlay_repaint(); // rides along with the content mark.

    let result = orch.paint_if_dirty();
    assert_eq!(
        result,
        PaintResult::Retry,
        "an all-failed attempt must retry"
    );

    // Recover. No new host raise — the retained work alone must drive it.
    stub.set_bulk_bridge_fail(false);
    let overlay_before = overlay_ops_len(&orch);
    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted, "recovery must commit");
    assert!(
        overlay_ops_len(&orch) > overlay_before,
        "the overlay mark raised alongside the failed content work must \
         survive the full-hold retry and repaint on recovery — got no new \
         overlay ops, meaning the mark was dropped"
    );
}

/// Damage counterpart of `held_slots_reuse_all_panes_failed_is_held_not_partial`
/// — same frozen-row fixture, both panes' own damaged row fails.
///
/// Today, `paint_damage_regime` has the identical unconditional
/// present-then-check-held ordering (and the same never-reset
/// `last_effective`) as the SlotsReuse arm above.
#[test]
fn held_damage_all_panes_failed_is_held_not_partial() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_presents = orch.grid_surface().presents();
    let overlay_presents = orch.overlay_surface().presents();

    stub.set_cell(1, 1, "top-edit");
    stub.set_cell(6, 1, "bottom-edit");
    stub.set_bulk_bridge_fail(true);
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 }); // TopRight's row.
    orch.mark_rows_damaged(0, RowSpan { r1: 6, r2: 6 }); // BottomRight's row.

    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Retry);
    assert_eq!(
        orch.grid_surface().presents(),
        grid_presents,
        "an all-failed Damage attempt is Held, not Partial: it must not present the grid"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        overlay_presents,
        "an all-failed Damage attempt must not present the overlay either"
    );
    assert_eq!(
        orch.last_trace().effective,
        None,
        "a fully held Damage attempt names no effective strategy"
    );
}

/// Fix-round regression (post Task 5 review): Damage counterpart of
/// `held_slots_reuse_full_hold_retries_the_complete_consumed_work` — a full
/// Damage hold must retry the complete consumed `work`, not just
/// `retry_for_held_rows(sheet, &spans)`'s row-scope reconstruction, for the
/// identical reason (a dropped `overlay` bit has no other chance to be
/// serviced). Same `with_show_selection(false)` discriminator: without it,
/// `show_selection` alone would force the recovery attempt's overlay to
/// paint regardless of whether the retry actually carried the mark.
#[test]
fn held_damage_full_hold_retries_the_complete_consumed_work() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2)
            .with_show_selection(false),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh baseline.

    stub.set_cell(1, 1, "top-edit");
    stub.set_cell(6, 1, "bottom-edit");
    stub.set_bulk_bridge_fail(true); // unconditional: both damaged rows fail.
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 }); // TopRight's row.
    orch.mark_rows_damaged(0, RowSpan { r1: 6, r2: 6 }); // BottomRight's row.
    orch.request_overlay_repaint(); // rides along with the row damage.

    let result = orch.paint_if_dirty();
    assert_eq!(
        result,
        PaintResult::Retry,
        "an all-failed Damage attempt must retry"
    );

    // Recover. No new host raise — the retained work alone must drive it.
    stub.set_bulk_bridge_fail(false);
    let overlay_before = overlay_ops_len(&orch);
    let result = orch.paint_if_dirty();

    assert_eq!(result, PaintResult::Painted, "recovery must commit");
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Damage),
        "a held Damage strip must retry as Damage, with its bands intact"
    );
    assert!(
        overlay_ops_len(&orch) > overlay_before,
        "the overlay mark raised alongside the failed row damage must \
         survive the full-hold retry and repaint on recovery — got no new \
         overlay ops, meaning the mark was dropped"
    );
}

/// Bullet 8 (RED against d8aed9c): a held Viewport attempt must name no
/// effective strategy — nothing actually committed pixels. Reuses the
/// `scroll_then_fail` fixture `held_viewport_presents_nothing_and_keeps_query_geometry`
/// already proves holds atomically on every other axis; this test adds the
/// one trace field that file doesn't check.
///
/// Today, `last_effective` is set to `Some(PaintRegimeTag::Viewport)` before
/// dispatch and is only ever overwritten by the `FreshFallback` arm — the
/// `Blitted`-then-`Held` early return in `paint_viewport_regime` never
/// resets it, so it survives unchanged into `last_trace.effective`.
#[test]
fn held_viewport_trace_names_no_effective_strategy() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(60)
            .with_active(5, 2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let result = scroll_then_fail(&stub, &mut orch);
    assert_eq!(result, PaintResult::Retry);

    assert_eq!(
        orch.last_trace().effective,
        None,
        "a held Viewport attempt names no effective strategy"
    );
}

/// Bullet 8 (GREEN today): a partial commit's trace already names the held
/// pane directly — `render_pane`'s bridge preflight stamps
/// `PaneVerdict::Held` on exactly the pane whose fetch failed, independent
/// of `Orchestrator`'s outcome-level bookkeeping. Confirms the per-pane
/// vocabulary Stage 4 can keep building on, using the same frozen-pane
/// fixture as `partial_commit_paints_healthy_panes_and_retries_held_panes`.
#[test]
fn partial_commit_trace_identifies_the_held_pane() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(1, 3, "top-edit"); // frozen band pane — healthy
    stub.set_cell(6, 3, "bottom-edit"); // scroll band pane — fails
    stub.set_bulk_bridge_fail_from(Some(3));
    orch.mark_content_dirty(PaneRegionMask::ALL);

    let result = orch.paint_if_dirty();
    assert_eq!(result, PaintResult::Retry);

    let trace = orch.last_trace();
    assert_eq!(
        trace.panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Held),
        "the failed pane's own verdict must name Held"
    );
    // A single safe-row edit in an already-primed pane is exactly the case
    // `paint_skip.rs`'s `row_band_repaint_paints_only_the_changed_row_band`
    // pins as the cheaper `Rows` plan, not `Full` — this assertion only
    // needs "actually painted, not held", so it accepts either real verdict
    // rather than assuming which one `plan_pane_repaint` picks.
    assert!(
        matches!(
            trace.panes[PaneRegion::TopRight as usize],
            Some(PaneVerdict::Full) | Some(PaneVerdict::Rows { .. })
        ),
        "the healthy pane must have actually painted, not held; got {:?}",
        trace.panes[PaneRegion::TopRight as usize]
    );
}
