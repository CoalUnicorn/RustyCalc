//! `Orchestrator<MemSurface>` five-regime integration test.
//!
//! Drives all five `PaintRegime` arms (`Fresh`, `SlotsReuse`, `Damage`,
//! `Viewport`, `Overlay`) through the same dispatch entry point a browser
//! would use, and asserts the captured `DrawOp` log matches each regime's
//! contract:
//!
//! - `Fresh`: full-canvas fill on the grid surface.
//! - `SlotsReuse`: no full-canvas fill (prior pixels are reused).
//! - `Damage`: row-band-clipped repaint, strictly fewer ops than SlotsReuse.
//! - `Viewport`: `DrawOp::Blit` ops on the grid surface (scroll-blit).
//! - `Overlay`: zero new grid ops; overlay surface clears + repaints.

#![allow(clippy::unwrap_used)]

mod common;

use std::rc::Rc;

use iron_canvas_core::RowSpan;
use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::geometry::constants::{
    CELL_AREA_INSET, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
    HEADER_ROW_HEIGHT,
};
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::types::coord::{AutofillTarget, RCRange, SheetArea};
use iron_canvas_core::{CanvasTheme, Orchestrator};
use iron_canvas_core::{PixelRect, Point};

use iron_canvas_core::{PaintRegimeTag, PaintResult, WorkFlags};
use iron_canvas_recorder::recording::{Frame, IcrHeader, Recording, ThemeSnapshot};
use iron_canvas_recorder::{DrawOp, MemSurface, RecorderPainter, RecordingSurface, replay};

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
fn grid_ops_since(orch: &Orchestrator<MemSurface>, cursor: usize) -> Vec<DrawOp> {
    orch.grid_surface().recorder().ops()[cursor..].to_vec()
}
fn overlay_ops_since(orch: &Orchestrator<MemSurface>, cursor: usize) -> Vec<DrawOp> {
    orch.overlay_surface().recorder().ops()[cursor..].to_vec()
}
fn grid_text_ops_containing(orch: &Orchestrator<MemSurface>, needle: &str) -> usize {
    orch.grid_surface()
        .recorder()
        .ops()
        .iter()
        .filter(|op| matches!(op, DrawOp::FillText { text, .. } if text.contains(needle)))
        .count()
}

#[test]
fn fresh_regime_emits_canvas_fill_and_overlay_clear() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));

    orch.paint_if_dirty();

    let grid_ops = orch.grid_surface().recorder().ops();
    let overlay_ops = orch.overlay_surface().recorder().ops();
    assert!(!grid_ops.is_empty(), "Fresh must paint the grid");
    assert!(!overlay_ops.is_empty(), "Fresh must paint the overlay");

    // The grid layer's full-canvas bg fill runs only on Fresh; SlotsReuse
    // / Blitted paths preserve prior pixels.
    assert!(
        grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "Fresh must emit a full-canvas RectFill (the grid bg)"
    );
    // Overlay clears its canvas at frame start (every regime that paints it).
    assert!(
        overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "Fresh must clear the overlay canvas"
    );
}

#[test]
fn slots_reuse_regime_skips_full_canvas_fill() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame.

    let grid_before = grid_ops_len(&orch);

    // A content-dirty signal keeps the viewport stable -> validity =
    // SlotsReuse. The decide cascade routes here because CONTENT blocks
    // the Viewport arm (blit on stale content is the recalc bug) and
    // validity stays SlotsReuse. Theme swaps no longer reach this regime
    // — they invalidate the paint cache and force Fresh.
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(!new_grid_ops.is_empty(), "SlotsReuse must repaint the grid");
    // No full-canvas bg fill: SlotsReuse path skips it by design.
    assert!(
        !new_grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "SlotsReuse must NOT emit a full-canvas RectFill"
    );
}

#[test]
fn viewport_regime_emits_blit_op() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);

    // Scroll one row. No content change. Raise OVERLAY (the only typed
    // signal we have for "something happened") so paint_if_dirty doesn't
    // bail empty — last_frame stays populated, decide() catches the
    // viewport shift via screen_for_blit and routes to Viewport.
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        new_grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::Blit { .. })),
        "Viewport regime must emit at least one DrawOp::Blit; got {:?}",
        new_grid_ops
    );
}

#[test]
fn overlay_regime_leaves_grid_untouched() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);

    // Autofill drag: raises OVERLAY only, no grid signal. Viewport
    // unchanged -> validity = SlotsReuse. decide() picks Overlay.
    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        new_grid_ops.is_empty(),
        "Overlay regime must NOT touch the grid surface; got {:?}",
        new_grid_ops
    );
    assert!(
        !new_overlay_ops.is_empty(),
        "Overlay regime must repaint the overlay"
    );
    assert!(
        new_overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "Overlay regime must clear the overlay canvas"
    );
}

#[test]
fn empty_signals_short_circuit_paint_if_dirty() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);

    // No signals raised since the prior paint — paint_if_dirty must bail.
    orch.paint_if_dirty();

    assert_eq!(grid_ops_len(&orch), grid_before);
    assert_eq!(overlay_ops_len(&orch), overlay_before);
}

#[test]
fn content_dirty_invalidates_pane_cache_through_slots_reuse() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let grid_before = grid_ops_len(&orch);

    // mark_content_dirty(ALL) raises CONTENT — viewport stays valid so
    // decide() picks SlotsReuse with mask = ALL.
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        !new_grid_ops.is_empty(),
        "Content-dirty SlotsReuse must repaint the grid"
    );
    assert!(
        !new_grid_ops.iter().any(|op| matches!(
            op,
            DrawOp::RectFill { rect, .. } if rect.width >= 800 && rect.height >= 600
        )),
        "Content-dirty SlotsReuse must NOT full-canvas-fill"
    );
}

/// Regression for the DEL-on-active-cell bug: a `CONTENT`-only signal must
/// repaint the overlay whenever there's an active cell, because the overlay
/// layer hosts the active-cell repaint hook that paints the model's current
/// value on top of the grid. Without this implication, `paint_slots_reuse_regime`
/// repaints the grid (correctly empty) but leaves the overlay's stale
/// active-cell pixels (the old value) on screen.
///
/// The fix lives in the `must_paint_overlay` predicate at the tail of
/// `paint_slots_reuse_regime`: `signals.overlay_dirty() || (CONTENT &&
/// selection.active_cell_repaint().is_some())`.
#[test]
fn content_dirty_with_active_cell_repaints_overlay() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(1, 1));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame and overlay.

    let overlay_before = overlay_ops_len(&orch);

    // DEL on the active cell: model value gone, only CONTENT raised
    // (no OVERLAY bit). Pre-fix, `if signals.overlay_dirty()` short-
    // circuited and the overlay's stale active-cell pixels stayed put.
    stub.set_cell(1, 1, "");
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();

    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        !new_overlay_ops.is_empty(),
        "CONTENT with an active cell must repaint the overlay; \
         got an empty overlay op stream — the stale active-cell pixels \
         would still be on screen"
    );
    assert!(
        new_overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "Overlay repaint must clear before redrawing; got {:?}",
        new_overlay_ops
    );
}

/// Regression for the workbook-switch stale-paint bug: `set_model` must
/// drop `last_frame` and raise `STRUCTURAL | OVERLAY` so the next paint
/// runs Fresh and clears both layers. Without this, swapping the
/// orchestrator's model in place (RustyCalc workbook switch, driven by
/// the `current_uuid` Effect in `worksheet.rs`) keeps the prior workbook's
/// chrome / pane geometry / cached pane buffers — and `decide()` routes
/// against the wrong `last_frame` to a stale SlotsReuse or Overlay paint
/// that never repaints the grid.
///
/// Tests the contract via the public `last_regime` accessor instead of
/// reaching into private fields: Fresh after `set_model` is exactly the
/// behavior the dropped `last_frame` + raised signals are supposed to
/// produce.
#[test]
fn set_model_drops_last_frame_and_forces_fresh() {
    let stub_a = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub_a));
    orch.paint_if_dirty(); // Fresh — primes last_frame.

    // A second paint with no signals would short-circuit; a content-dirty
    // paint here would land on SlotsReuse. We're proving set_model defeats
    // that path even with a steady viewport / sheet / freeze / size.
    let stub_b = Rc::new(TestModel::synthetic_grid());
    orch.set_model(stub_b.clone());

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);
    orch.paint_if_dirty();

    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "set_model must drop last_frame so the next paint takes Fresh; \
         got {:?} — the workbook-switch stale-paint bug is back",
        orch.last_regime(),
    );

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        !new_grid_ops.is_empty(),
        "set_model must raise STRUCTURAL so Fresh repaints the grid"
    );
    assert!(
        !new_overlay_ops.is_empty(),
        "set_model must raise OVERLAY so Fresh repaints the overlay \
         — else the prior workbook's selection / autofill / clipboard \
         pixels persist on screen"
    );
}

// ─── Stage 5: Recording round-trip through RecordingSurface<MemSurface> ───
//
// Reuses TestModel + the same regime scenarios as the MemSurface
// suite above. Each test wraps both surfaces in RecordingSurface and
// asserts:
//   1. Orchestrator::last_regime() stamps the expected PaintRegimeTag.
//      (Covers the deferred-from-Stage-2 `last_regime_set_after_each_arm`.)
//   2. The captured per-frame op streams round-trip through serde +
//      replay() byte-equal against the originals.

fn build_rec(model: Rc<TestModel>) -> Orchestrator<RecordingSurface<MemSurface>> {
    let grid = RecordingSurface::new(MemSurface::new());
    let overlay = RecordingSurface::new(MemSurface::new());
    grid.enable_recording();
    overlay.enable_recording();
    let mut orch = Orchestrator::<RecordingSurface<MemSurface>>::new(grid, overlay);
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_model(model);
    orch
}

/// Bracket a paint with begin_frame/end_frame on both surfaces and
/// return (grid_ops, overlay_ops, regime, work_bits). The `.icr` v3
/// `signals: u8` field is fed from `WorkFlags::bits()`, whose layout is
/// pinned to the `GridSignals` word it replaced.
fn paint_and_capture(
    orch: &mut Orchestrator<RecordingSurface<MemSurface>>,
) -> (Vec<DrawOp>, Vec<DrawOp>, Option<PaintRegimeTag>, u8) {
    orch.grid_surface().begin_frame();
    orch.overlay_surface().begin_frame();
    orch.paint_if_dirty();
    let grid_ops = orch.grid_surface().end_frame();
    let overlay_ops = orch.overlay_surface().end_frame();
    (
        grid_ops,
        overlay_ops,
        orch.last_regime(),
        orch.last_work_flags().bits(),
    )
}

/// `replay()` prepends one `DrawOp::InvalidateCache` to keep the sink's
/// ctx-state cache from drifting. Trim it so the tail is comparable to
/// the originally captured stream.
fn replay_and_drain(ops: &[DrawOp]) -> Vec<DrawOp> {
    let sink = RecorderPainter::new();
    replay(&sink, ops);
    let mut replayed = sink.into_ops();
    // Drop the synthetic leading InvalidateCache prepended by replay().
    assert!(matches!(replayed.first(), Some(DrawOp::InvalidateCache)));
    replayed.remove(0);
    replayed
}

#[test]
fn last_regime_fresh_after_initial_paint() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    let (grid_ops, overlay_ops, regime, _) = paint_and_capture(&mut orch);

    assert_eq!(regime, Some(PaintRegimeTag::Fresh));
    assert!(!grid_ops.is_empty());
    assert!(!overlay_ops.is_empty());
    // Round-trip through replay — proves the captured stream is
    // structurally valid even before serde gets involved.
    assert_eq!(replay_and_drain(&grid_ops), grid_ops);
    assert_eq!(replay_and_drain(&overlay_ops), overlay_ops);
}

#[test]
fn last_regime_fresh_after_theme_swap() {
    // Theme is frame-wide: a palette change makes the cached frame's
    // pixels stale. `is_still_valid` rejects the theme-mismatched frame
    // (and `set_theme` invalidates the content-keyed paint cache), so the
    // next paint takes the Fresh arm; SlotsReuse would repaint stale-color
    // cells under fresh chrome.
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    orch.set_theme(CanvasTheme::dark());
    let (grid_ops, _, regime, _) = paint_and_capture(&mut orch);

    assert_eq!(regime, Some(PaintRegimeTag::Fresh));
    assert!(!grid_ops.is_empty());
}

#[test]
fn last_regime_viewport_after_row_scroll() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    stub.set_top_row(2);
    orch.request_overlay_repaint();
    let (grid_ops, _, regime, _) = paint_and_capture(&mut orch);

    assert_eq!(regime, Some(PaintRegimeTag::Viewport));
    assert!(grid_ops.iter().any(|op| matches!(op, DrawOp::Blit { .. })));
}

#[test]
fn last_regime_overlay_after_autofill_drag() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    let (grid_ops, overlay_ops, regime, _) = paint_and_capture(&mut orch);

    assert_eq!(regime, Some(PaintRegimeTag::Overlay));
    assert!(grid_ops.is_empty(), "Overlay must not touch the grid");
    assert!(!overlay_ops.is_empty());
}

#[test]
fn recording_serde_round_trip_across_all_five_regimes() {
    // Drive Fresh -> Damage -> SlotsReuse -> Viewport -> Overlay through one
    // Orchestrator, collecting one Frame per regime, then serialize the
    // whole Recording and assert deserialize is bit-equal to the original.
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));

    let mut frames: Vec<Frame> = Vec::new();
    let mut push = |orch: &mut Orchestrator<RecordingSurface<MemSurface>>, t_ms: u64| {
        let (grid_ops, overlay_ops, regime, signals) = paint_and_capture(orch);
        // Skip idle frames so the recording matches the production
        // paint_if_dirty drop-empty-frames behavior.
        if grid_ops.is_empty() && overlay_ops.is_empty() {
            return;
        }
        let idx = frames.len() as u32;
        frames.push(Frame {
            frame_idx: idx,
            t_ms,
            regime: regime.expect("regime must be stamped"),
            signals,
            grid_ops,
            overlay_ops,
        });
    };

    push(&mut orch, 0); // Fresh
    // mark_rows_damaged names row 2 on sheet 0 with an actual edit behind it:
    // viewport stays reusable and every CONTENT raise since the last paint
    // named its rows on the on-screen sheet -> Damage.
    stub.set_cell(2, 1, "changed");
    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    push(&mut orch, 8); // Damage
    // mark_content_dirty raises CONTENT; viewport stays valid -> SlotsReuse.
    // (set_theme used to land here too, but a palette change now invalidates
    // the paint cache and routes to Fresh, so it can't be used as a
    // SlotsReuse trigger.)
    orch.mark_content_dirty(PaneRegionMask::ALL);
    push(&mut orch, 16); // SlotsReuse
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    push(&mut orch, 32); // Viewport
    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    push(&mut orch, 48); // Overlay

    assert_eq!(frames.len(), 5, "all five regimes should produce frames");
    let regimes: Vec<_> = frames.iter().map(|f| f.regime).collect();
    assert_eq!(
        regimes,
        vec![
            PaintRegimeTag::Fresh,
            PaintRegimeTag::Damage,
            PaintRegimeTag::SlotsReuse,
            PaintRegimeTag::Viewport,
            PaintRegimeTag::Overlay,
        ],
    );

    let header = IcrHeader::new(
        800.0,
        600.0,
        1.0,
        ThemeSnapshot::from(orch.theme()),
        0, // deterministic — tests don't read wall-clock for this field
    );
    let mut recording = Recording::new(header);
    for f in &frames {
        recording.push_frame(f.clone());
    }

    let bytes = recording.serialize().expect("serialize");
    let back = Recording::deserialize(&bytes).expect("deserialize");
    assert_eq!(recording, back, "Recording must survive a serde round-trip");

    // Per-frame: replay each frame's op stream into a fresh sink and
    // assert byte-equal against the deserialized stream.
    for (orig, restored) in frames.iter().zip(&back.frames) {
        assert_eq!(orig.grid_ops, restored.grid_ops);
        assert_eq!(orig.overlay_ops, restored.overlay_ops);
        assert_eq!(replay_and_drain(&restored.grid_ops), orig.grid_ops);
        assert_eq!(replay_and_drain(&restored.overlay_ops), orig.overlay_ops);
    }
}

// ─── Damage regime ───

#[test]
fn damage_regime_paints_far_less_than_slots_reuse() {
    // Same model, same content signal — one orchestrator takes the mask
    // path, one the damage path. The band repaint being strictly smaller
    // is the entire point of the regime.
    //
    // The edit below must be load-bearing in TWO ways now:
    //
    // 1. (pre-Task-4 reason, still true) An *unedited* mask=ALL content
    //    signal fetches identical bytes and fingerprint-skips the entire
    //    SlotsReuse walk for free (0 cell ops) — which would make
    //    SlotsReuse cheaper than Damage's unconditional named-row repaint
    //    no matter the grid size. A real edit is what actually raises
    //    CONTENT in production.
    // 2. (Task 4) `render_pane`'s SlotsReuse mismatch now ALSO clips to a
    //    row-band repaint when `plan_pane_repaint` finds the change safe —
    //    a single-row edit like the old `stub.set_cell(2, 1, ...)` fixture
    //    would make SlotsReuse pay for the SAME clipped one-row band
    //    Damage takes, collapsing the very gap this test exists to prove.
    //    Nine widely-spread row edits exceed `plan_pane_repaint`'s merge
    //    cap (`MAX_DAMAGE_SPANS`), forcing SlotsReuse's mismatch back onto
    //    the unclipped whole-pane walk this comparison is meant to
    //    contrast against Damage's still-clipped single-row band (Damage
    //    paints only the row named in `mark_rows_damaged` below,
    //    regardless of how many rows actually changed).
    let stub = Rc::new(TestModel::synthetic_grid());

    let mut slots = build(Rc::clone(&stub));
    slots.paint_if_dirty();
    let mut damage = build(Rc::clone(&stub));
    damage.paint_if_dirty();

    for row in [2, 5, 8, 11, 14, 17, 20, 23, 26] {
        stub.set_cell(row, 1, "changed");
    }

    let before = grid_ops_len(&slots);
    slots.mark_content_dirty(PaneRegionMask::ALL);
    slots.paint_if_dirty();
    let slots_ops = grid_ops_len(&slots) - before;

    let before = grid_ops_len(&damage);
    damage.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    damage.paint_if_dirty();
    let damage_ops = grid_ops_len(&damage) - before;

    assert_eq!(damage.last_regime(), Some(PaintRegimeTag::Damage));
    assert!(damage_ops > 0, "damage must repaint the band");
    assert!(
        damage_ops * 3 < slots_ops,
        "one-row band repaint must be far smaller than the full SlotsReuse walk (got {damage_ops} vs {slots_ops})"
    );
}

#[test]
fn damage_regime_repaints_chrome_like_other_grid_regimes() {
    // The damage renderer must run the full grid wrapper: frozen
    // separators paint after cells (winning back boundary pixels the
    // band re-stroked) and headers/corner stay in the sequence. Group
    // markers are the observable contract.
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let before = grid_ops_len(&orch);
    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.paint_if_dirty();
    let ops = grid_ops_since(&orch, before);

    for class in [
        GroupClass::Grid,
        GroupClass::Cells,
        GroupClass::FrozenSep,
        GroupClass::Headers,
    ] {
        assert!(
            ops.iter()
                .any(|op| matches!(op, DrawOp::BeginGroup { class: c } if *c == class)),
            "damage paint must emit the {class:?} group like every grid regime"
        );
    }
}

#[test]
fn plain_content_dirty_poisons_damage_to_slots_reuse() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
}

#[test]
fn cross_sheet_damage_degrades_to_slots_reuse() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.mark_rows_damaged(1, RowSpan { r1: 3, r2: 3 });
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
}

#[test]
fn damage_is_drained_by_the_paint() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.paint_if_dirty();
    let after = grid_ops_len(&orch);
    orch.paint_if_dirty(); // nothing raised since -> no-op
    assert_eq!(grid_ops_len(&orch), after);
}

// ─── present() contract ───

/// Every regime arm must call `Surface::present()` exactly once per layer
/// it actually painted, and must NOT present a layer it left untouched
/// (Overlay regime skips the grid entirely). `MemSurface::presents()`
/// counts real `present()` calls — this is the counting test Task 2's
/// `WebSurface` back-buffer flip depends on.
#[test]
fn every_paint_arm_presents_the_surfaces_it_painted() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));

    // Fresh: paints grid + overlay -> one present each.
    orch.paint_if_dirty();
    assert_eq!(orch.grid_surface().presents(), 1, "Fresh presents the grid");
    assert_eq!(
        orch.overlay_surface().presents(),
        1,
        "Fresh presents the overlay"
    );

    // Overlay regime: grid pixels untouched -> grid must NOT re-present.
    orch.request_overlay_repaint();
    orch.paint_if_dirty();
    assert_eq!(
        orch.grid_surface().presents(),
        1,
        "Overlay regime must not present the grid"
    );
    assert_eq!(
        orch.overlay_surface().presents(),
        2,
        "Overlay regime presents the overlay"
    );

    // SlotsReuse (content dirty, viewport stable): grid presents again.
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();
    assert_eq!(
        orch.grid_surface().presents(),
        2,
        "SlotsReuse presents the grid"
    );

    // Viewport (scroll one row): grid blit + overlay repaint -> both present.
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();
    assert_eq!(
        orch.grid_surface().presents(),
        3,
        "Viewport presents the grid"
    );
}

/// Slice out the ops painted strictly between one `GroupClass::Cells`
/// `BeginGroup` and its matching `EndGroup` — the cell band only, excluding
/// the headers/frozen-separator ops that repaint every Damage frame
/// regardless and would otherwise mask a strip's atomic no-op. Cells never
/// nests another group inside it, so the first `EndGroup` after the marker
/// closes it.
fn cells_group_ops(ops: &[DrawOp]) -> Vec<DrawOp> {
    let start = ops
        .iter()
        .position(|op| matches!(op, DrawOp::BeginGroup { class } if *class == GroupClass::Cells))
        .expect("ops must contain a Cells group");
    let end = ops[start + 1..]
        .iter()
        .position(|op| matches!(op, DrawOp::EndGroup))
        .expect("Cells group must close");
    ops[start + 1..start + 1 + end].to_vec()
}

/// Task 5, acceptance criterion 2, at the orchestrator wiring level (the
/// unit-level proof lives in `row_fingerprint_repaint.rs`'s
/// `lifecycle_bridge_failed_damage_strip_is_atomic`): a transient
/// `BridgeFailed` on the Damage regime's strip fetch must paint literally
/// zero ops inside the `Cells` group — the atomic preflight rejects the
/// whole strip before any splice/clear/paint. Filtered to the `Cells` group
/// specifically because headers and the frozen separator still repaint
/// every Damage frame regardless of the strip's outcome.
#[test]
fn damage_regime_bridge_failure_paints_no_cell_ops() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes the pane cache + painted tree.

    stub.set_value_bridge_fail(true);
    let grid_before = grid_ops_len(&orch);
    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.paint_if_dirty();
    let new_grid_ops = grid_ops_since(&orch, grid_before);

    assert!(
        cells_group_ops(&new_grid_ops).is_empty(),
        "a BridgeFailed Damage strip must paint zero ops inside the Cells \
         group; got {:?}",
        new_grid_ops
    );

    // The atomic-skip path must not poison future frames: once the bridge
    // recovers, Damage must repaint the edited cell normally.
    stub.set_value_bridge_fail(false);
    stub.set_cell(2, 1, "changed");
    let grid_before = grid_ops_len(&orch);
    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.paint_if_dirty();
    let new_grid_ops = grid_ops_since(&orch, grid_before);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Damage));
    assert!(
        !cells_group_ops(&new_grid_ops).is_empty(),
        "Damage must repaint the Cells group normally on the next \
         successful frame after a BridgeFailed strip"
    );
}

#[test]
fn damage_with_active_cell_repaints_overlay() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(1, 1));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let overlay_before = overlay_ops_len(&orch);
    stub.set_cell(1, 1, "");
    orch.mark_rows_damaged(0, RowSpan { r1: 1, r2: 1 });
    orch.paint_if_dirty();

    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Damage));
    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        new_overlay_ops
            .iter()
            .any(|op| matches!(op, DrawOp::ClearRect { .. })),
        "CONTENT via Damage with an active cell must clear and repaint the overlay"
    );
}

/// The scrollable pane starts after the frozen bands, not at the canvas
/// origin. Hosts that trigger on the pane's edges (autoscroll while
/// dragging a selection) read this rect; measuring against the canvas puts
/// the near edges `frozen_offset` px inside the frozen band, where nothing
/// scrolls.
#[test]
fn scroll_pane_rect_starts_after_the_frozen_bands() {
    let mut orch = build(Rc::new(TestModel::new().with_frozen(1, 3)));
    assert_eq!(
        orch.scroll_pane_rect(),
        None,
        "no painted frame yet — the host must not autoscroll pre-mount"
    );

    orch.paint_if_dirty();

    let x = (f64::from(HEADER_COL_WIDTH + CELL_AREA_INSET)
        + 3.0 * DEFAULT_COL_WIDTH
        + f64::from(FROZEN_SEP))
    .round() as i32;
    let y = (f64::from(HEADER_ROW_HEIGHT + CELL_AREA_INSET)
        + DEFAULT_ROW_HEIGHT
        + f64::from(FROZEN_SEP))
    .round() as i32;
    assert_eq!(
        orch.scroll_pane_rect(),
        Some(PixelRect {
            top_left: Point { x, y },
            width: 800 - x,
            height: 600 - y,
        })
    );
}

/// Frozen bands can exceed the canvas. The pane collapses to zero extent
/// rather than reporting a negative one — a negative width/height would flow
/// straight into the host's scroll-window budget and edge-scroll thresholds.
#[test]
fn scroll_pane_rect_collapses_to_zero_when_frozen_bands_exceed_the_canvas() {
    let mut orch = build(Rc::new(TestModel::new().with_frozen(400, 400)));
    orch.paint_if_dirty();

    let Some(rect) = orch.scroll_pane_rect() else {
        unreachable!("a painted frame must yield a rect");
    };
    assert_eq!((rect.width, rect.height), (0, 0));
    assert!(rect.top_left.x > 800 && rect.top_left.y > 600);
}

// ─── Stage 2: dispatch over one `PendingWork` value ───
//
// Every assertion below drives the real `paint_if_dirty` entry point and
// reads the regime back through `last_regime()` / `last_work_flags()` / the
// recorded op stream. None of them inspect `PendingWork` directly — the
// point is to pin the *dispatch* the work algebra produces, which is what a
// regression would actually break.

/// Overlay setters value-compare before marking. A setter re-called with the
/// value it already holds must queue nothing at all, or every idle rAF tick
/// that re-pushes an unchanged reactive memo would repaint the overlay.
#[test]
fn unchanged_overlay_setters_queue_no_work() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame.

    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    // Same autofill target, and the three others at their existing default.
    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    orch.set_clipboard(None);
    orch.set_point_range(None);
    orch.set_formula_refs(Vec::new());

    assert_eq!(
        orch.paint_if_dirty(),
        PaintResult::Idle,
        "no overlay value changed, so nothing may be queued"
    );
}

/// The mirror of the test above: each overlay setter that *does* change
/// state dispatches the cheapest arm, never a grid repaint.
#[test]
fn changed_overlay_setters_dispatch_overlay() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    // Each setter is checked in its own paint, so a later one cannot mask an
    // earlier one that queued nothing.
    let expect_overlay_only = |orch: &mut Orchestrator<MemSurface>, name: &str| {
        let grid_before = grid_ops_len(orch);
        assert_eq!(orch.paint_if_dirty(), PaintResult::Painted, "{name}");
        assert_eq!(
            orch.last_regime(),
            Some(PaintRegimeTag::Overlay),
            "{name} must dispatch Overlay"
        );
        assert!(
            grid_ops_since(orch, grid_before).is_empty(),
            "{name} must not touch the grid surface"
        );
    };

    orch.set_extend_to(Some(AutofillTarget { row: 1, col: 2 }));
    expect_overlay_only(&mut orch, "set_extend_to");

    orch.set_clipboard(Some(SheetArea {
        sheet: 0,
        range: RCRange {
            r1: 1,
            c1: 1,
            r2: 2,
            c2: 2,
        },
    }));
    expect_overlay_only(&mut orch, "set_clipboard");

    orch.set_point_range(Some(RCRange {
        r1: 3,
        c1: 1,
        r2: 3,
        c2: 4,
    }));
    expect_overlay_only(&mut orch, "set_point_range");

    orch.request_overlay_repaint();
    expect_overlay_only(&mut orch, "request_overlay_repaint");
}

/// `request_repaint` escalates to `Fresh` but must PRESERVE content work
/// already queued in the same tick rather than clearing it, so the attempt
/// that reaches `decide` still declares itself content-carrying.
///
/// The `CONTENT` flag is the discriminating assertion here: today's `Fresh`
/// re-reads every pane unconditionally, so a cleared content mark paints
/// the same pixels and no output-level assertion can see the difference.
/// It becomes load-bearing the moment `Fresh` learns to adopt a
/// range-matched buffer — which is exactly why the mark must not be
/// dropped now.
#[test]
fn content_then_request_repaint_is_fresh_and_keeps_the_content_mark() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame and the pane cache.

    stub.set_cell(2, 1, "edited");
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.request_repaint();

    stub.reset_bulk_fetch_calls();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert!(
        orch.last_work_flags()
            .contains(WorkFlags::CONTENT | WorkFlags::GEOMETRY),
        "request_repaint must ADD geometry work, not replace the queued \
         content work; got {:?}",
        orch.last_work_flags()
    );
    assert!(
        stub.bulk_fetch_calls() > 0,
        "the escalated frame must still read pane content from the model"
    );
    assert!(
        grid_text_ops_containing(&orch, "edited") > 0,
        "the edit queued before request_repaint must still reach the screen"
    );
}

/// `set_model` replaces model identity, so it discards work belonging to the
/// old model and installs the worst-case value. Repeating it must keep
/// forcing `Fresh` — there is no `Rc::ptr_eq` dedupe (current contract).
#[test]
fn repeated_set_model_still_forces_fresh() {
    let stub_a = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub_a));
    orch.paint_if_dirty();

    for _ in 0..2 {
        orch.set_model(Rc::new(TestModel::synthetic_grid()));
        assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
        assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    }
}

/// THE dispatch hazard this stage had to get right.
///
/// An arrow-key move to a cell already on screen changes no scroll, freeze,
/// sheet, size or cell value — only the view. `decide` runs the geometric
/// `screen_for_blit` probe first, gets `None` (no pixels move), and must
/// then let `Overlay` match *while ignoring the view mark*.
///
/// Expressing `Overlay`'s guard as the mechanical port of the old
/// `!signals.grid_dirty()` — whose bit group already covered the then-dead
/// `VIEWPORT` bit — would make this row of the dispatch matrix unreachable
/// and turn the single most common interaction in the app into a full-grid
/// repaint. That regression is silent in every other test; this is the one
/// that fails.
#[test]
fn visible_selection_navigation_dispatches_overlay() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame.

    stub.set_active(6, 2); // still well inside the painted viewport
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Overlay),
        "view work with no pixel shift on a reusable frame must fall back to \
         Overlay; anything else means the Overlay guard is excluding `view`"
    );
    assert!(
        orch.last_work_flags().contains(WorkFlags::VIEW),
        "the attempt really did carry view work — the Overlay verdict is the \
         fallback, not a missing mark"
    );
}

/// Companion to the test above, stated as the cost that actually matters: a
/// selection move inside the viewport must emit zero new grid operations.
#[test]
fn view_only_navigation_without_a_shift_emits_no_grid_ops() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);

    stub.set_active(6, 2);
    orch.view_changed();
    orch.paint_if_dirty();

    assert!(
        grid_ops_since(&orch, grid_before).is_empty(),
        "in-viewport navigation must not repaint the grid; got {:?}",
        grid_ops_since(&orch, grid_before)
    );
    assert!(
        !overlay_ops_since(&orch, overlay_before).is_empty(),
        "...but it must move the selection rectangle on the overlay"
    );
}

/// The other view row: when the movement *does* shift pixels, the geometric
/// probe claims it and `Viewport` blits the kept band.
#[test]
fn real_scroll_view_change_dispatches_viewport() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_before = grid_ops_len(&orch);
    stub.set_top_row(2);
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));
    assert!(
        grid_ops_since(&orch, grid_before)
            .iter()
            .any(|op| matches!(op, DrawOp::Blit { .. })),
        "a real shift must blit the kept band"
    );
}

/// `decide`'s `Viewport` probe carries an explicit `!work.has_geometry()`
/// guard. It has no reachable failure through public API today: every
/// current geometry producer (`resize` here) already drops `last_frame`
/// before `decide` runs, which excludes the probe on its own. This pins the
/// externally observable contract instead of the guard specifically —
/// geometry work concurrent with a real shift must still land on `Fresh`,
/// never `Viewport` — so it keeps failing the moment a future geometry
/// producer stops tripping last_frame/size/theme independently.
#[test]
fn geometry_plus_real_scroll_never_dispatches_viewport() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_top_row(2); // the same real shift real_scroll_view_change_dispatches_viewport uses
    orch.resize(CanvasSize { w: 900.0, h: 600.0 }, 1.0); // marks geometry
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "geometry work concurrent with a real shift must dispatch Fresh, \
         never Viewport"
    );
    assert!(
        orch.last_work_flags()
            .contains(WorkFlags::GEOMETRY | WorkFlags::VIEW),
        "the attempt really did carry both geometry and view work; got {:?}",
        orch.last_work_flags()
    );
}

/// Row work whose sheet tag doesn't match the painted frame can't clip to
/// bands, so it falls back to `SlotsReuse` — and the fallback mask must be
/// `ALL`, never a mask derived from where the spans happen to intersect the
/// visible panes. Proven by editing both a frozen-band and a scroll-band
/// cell: a narrowed mask would leave one of them unfetched and unpainted.
#[test]
fn row_work_ineligible_for_damage_falls_back_to_all_panes() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen_rows(2),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(1, 3, "frozen-edit"); // top band
    stub.set_cell(6, 3, "scroll-edit"); // bottom band
    // Rows recorded against a sheet that is not the one on screen: `Damage`
    // is ineligible, but the content work is still real.
    orch.mark_rows_damaged(7, RowSpan { r1: 1, r2: 1 });

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
    assert!(
        grid_text_ops_containing(&orch, "frozen-edit") > 0,
        "SlotsReuse(ALL) must refetch the frozen-band panes"
    );
    assert!(
        grid_text_ops_containing(&orch, "scroll-edit") > 0,
        "SlotsReuse(ALL) must refetch the scroll-band panes"
    );
}

/// A sheet switch is a view change whose scroll, freeze and canvas size are
/// all identical — exactly the shape that the cheap arms would happily
/// reuse. It must reach `Fresh` and re-read the model, because pane buffers
/// are keyed on row/column range with no sheet identity: reusing the frame
/// would keep sheet 0's values on screen under sheet 1's header.
///
/// The fetch-count assertion discriminates against the real failure mode
/// (`Overlay` or `SlotsReuse` silently claiming the switch, which reads
/// nothing / reads under the old frame). It does not isolate
/// `paint_fresh_regime`'s pane-cache invalidation, which is unobservable
/// today — see the note on that method.
#[test]
fn active_sheet_view_change_is_fresh_and_refetches_pane_content() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh on sheet 0 — fills the pane cache.

    stub.set_sheet(1); // identical visible coordinates
    orch.view_changed();
    stub.reset_bulk_fetch_calls();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "a sheet switch invalidates the frame outright"
    );
    assert!(
        stub.bulk_fetch_calls() > 0,
        "the new sheet's pane content must be read from the model, not \
         carried over from the previous sheet's frame"
    );
}

/// Commit-then-move (Enter/Tab): the one real dual-effect producer. Stage 2
/// keeps it conservative — content plus view is always `Fresh`, never a blit
/// over changed values and never a band-clipped `Damage`.
#[test]
fn content_plus_view_is_fresh() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(5, 1));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    stub.set_cell(5, 1, "typed");
    stub.set_active(6, 1);
    orch.mark_rows_damaged(0, RowSpan { r1: 5, r2: 5 });
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(
        orch.last_regime(),
        Some(PaintRegimeTag::Fresh),
        "content + view must not clip to bands or blit"
    );
    assert!(grid_text_ops_containing(&orch, "typed") > 0);
}
