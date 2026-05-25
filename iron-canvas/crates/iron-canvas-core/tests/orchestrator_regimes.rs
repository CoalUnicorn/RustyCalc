//! `Orchestrator<MemSurface, _>` four-regime integration test.
//!
//! Drives all four `PaintRegime` arms (`Fresh`, `SlotsReuse`, `Viewport`,
//! `Overlay`) through the same dispatch entry point a browser would use,
//! and asserts the captured `DrawOp` log matches each regime's contract:
//!
//! - **Fresh**: full-canvas fill on the grid surface.
//! - **SlotsReuse**: no full-canvas fill (prior pixels are reused).
//! - **Viewport**: `DrawOp::Blit` ops on the grid surface (scroll-blit).
//! - **Overlay**: zero new grid ops; overlay surface clears + repaints.

#![allow(clippy::unwrap_used)]

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::PaneRegionMask;
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::types::coord::AutofillTarget;
use iron_canvas_core::{CanvasModel, CanvasTheme, Orchestrator};

use iron_canvas_core::PaintRegimeTag;
use iron_canvas_recorder::recording::{Frame, IcrHeader, Recording, ThemeSnapshot};
use iron_canvas_recorder::{replay, DrawOp, MemSurface, RecorderPainter, RecordingSurface};

use common::TestModel;

fn build(model: Rc<TestModel>) -> Orchestrator<MemSurface, Rc<TestModel>> {
    let mut orch =
        Orchestrator::<MemSurface, Rc<TestModel>>::new(MemSurface::new(), MemSurface::new());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1);
    orch.set_model(model);
    orch
}

fn grid_ops_len<M: CanvasModel>(orch: &Orchestrator<MemSurface, M>) -> usize {
    orch.grid_surface().recorder().ops().len()
}
fn overlay_ops_len<M: CanvasModel>(orch: &Orchestrator<MemSurface, M>) -> usize {
    orch.overlay_surface().recorder().ops().len()
}
fn grid_ops_since<M: CanvasModel>(
    orch: &Orchestrator<MemSurface, M>,
    cursor: usize,
) -> Vec<DrawOp> {
    orch.grid_surface().recorder().ops()[cursor..].to_vec()
}
fn overlay_ops_since<M: CanvasModel>(
    orch: &Orchestrator<MemSurface, M>,
    cursor: usize,
) -> Vec<DrawOp> {
    orch.overlay_surface().recorder().ops()[cursor..].to_vec()
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

    // A content-dirty signal keeps the viewport stable → validity =
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
    // unchanged → validity = SlotsReuse. decide() picks Overlay.
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

// ─── Stage 5: Recording round-trip through RecordingSurface<MemSurface> ───
//
// Reuses TestModel + the same regime scenarios as the MemSurface
// suite above. Each test wraps both surfaces in RecordingSurface and
// asserts:
//   1. Orchestrator::last_regime() stamps the expected PaintRegimeTag.
//      (Covers the deferred-from-Stage-2 `last_regime_set_after_each_arm`.)
//   2. The captured per-frame op streams round-trip through serde +
//      replay() byte-equal against the originals.

fn build_rec(
    model: Rc<TestModel>,
) -> Orchestrator<RecordingSurface<MemSurface>, Rc<TestModel>> {
    let grid = RecordingSurface::new(MemSurface::new());
    let overlay = RecordingSurface::new(MemSurface::new());
    grid.enable_recording();
    overlay.enable_recording();
    let mut orch = Orchestrator::<RecordingSurface<MemSurface>, Rc<TestModel>>::new(
        grid, overlay,
    );
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1);
    orch.set_model(model);
    orch
}

/// Bracket a paint with begin_frame/end_frame on both surfaces and
/// return (grid_ops, overlay_ops, regime, signals_bits).
fn paint_and_capture(
    orch: &mut Orchestrator<RecordingSurface<MemSurface>, Rc<TestModel>>,
) -> (Vec<DrawOp>, Vec<DrawOp>, Option<PaintRegimeTag>, u8) {
    orch.grid_surface().begin_frame();
    orch.overlay_surface().begin_frame();
    orch.paint_if_dirty();
    let grid_ops = orch.grid_surface().end_frame();
    let overlay_ops = orch.overlay_surface().end_frame();
    (grid_ops, overlay_ops, orch.last_regime(), orch.last_signals().bits())
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
    // Theme is frame-wide: the per-cell paint cache and last_frame's
    // theme snapshot both go stale on a palette change, so set_theme
    // drops last_frame and invalidates the paint cache. The next paint
    // takes the Fresh arm; SlotsReuse would repaint stale-color cells
    // under fresh chrome.
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
fn recording_serde_round_trip_across_all_four_regimes() {
    // Drive Fresh → SlotsReuse → Viewport → Overlay through one
    // Orchestrator, collecting one Frame per regime, then serialize the
    // whole Recording and assert deserialize is bit-equal to the original.
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));

    let mut frames: Vec<Frame> = Vec::new();
    let mut push = |orch: &mut Orchestrator<
        RecordingSurface<MemSurface>,
        Rc<TestModel>,
    >,
                    t_ms: u64| {
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
    // mark_content_dirty raises CONTENT; viewport stays valid → SlotsReuse.
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

    assert_eq!(frames.len(), 4, "all four regimes should produce frames");
    let regimes: Vec<_> = frames.iter().map(|f| f.regime).collect();
    assert_eq!(
        regimes,
        vec![
            PaintRegimeTag::Fresh,
            PaintRegimeTag::SlotsReuse,
            PaintRegimeTag::Viewport,
            PaintRegimeTag::Overlay,
        ],
    );

    let header = IcrHeader::new(
        800.0,
        600.0,
        1,
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
