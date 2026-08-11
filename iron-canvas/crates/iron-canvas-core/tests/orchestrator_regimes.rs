//! `Orchestrator<MemSurface>` five-regime integration test.
//!
//! Drives all five strategies a `FramePlan` can select (`Fresh`,
//! `SlotsReuse`, `Damage`, `Viewport`, `Overlay`) through the same dispatch
//! entry point a browser would use, and asserts the captured `DrawOp` log
//! matches each regime's contract:
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
use iron_canvas_core::chrome::{PaneRegion, PaneRegionMask};
use iron_canvas_core::geometry::CanvasSize;
use iron_canvas_core::geometry::constants::{
    CELL_AREA_INSET, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP, HEADER_COL_WIDTH,
    HEADER_ROW_HEIGHT,
};
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::types::coord::{AutofillTarget, RCRange, SheetArea};
use iron_canvas_core::{CanvasTheme, Orchestrator};
use iron_canvas_core::{PixelRect, Point};

use iron_canvas_core::{PaintRegimeTag, PaintResult, PaneVerdict, WorkFlags};
use iron_canvas_recorder::recording::{
    Frame, IcrHeader, RecordOrigin, RecordedPaintResult, Recording, ThemeSnapshot, TraceRecord,
};
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

/// Occurrences of a fieldless `DrawOp` (Stage 6 counts `InvalidateCache`
/// and `ResetTextDefaults` this way).
fn count_op(ops: &[DrawOp], needle: &DrawOp) -> usize {
    ops.iter().filter(|op| *op == needle).count()
}

/// Index of the first op that puts pixels on the surface. Painter *state*
/// ops (cache/text-default resets, DPR transform), group brackets and clip
/// pushes are transparent here — Gate A's claim is about ordering against
/// drawing, not against bookkeeping.
fn first_draw_index(ops: &[DrawOp]) -> Option<usize> {
    ops.iter().position(|op| {
        !matches!(
            op,
            DrawOp::InvalidateCache
                | DrawOp::ResetTextDefaults
                | DrawOp::ApplyDprTransform { .. }
                | DrawOp::BeginGroup { .. }
                | DrawOp::EndGroup
                | DrawOp::PushClip { .. }
                | DrawOp::PopClip
        )
    })
}

/// Color of a Fresh frame's full-canvas background fill — the first draw
/// `Layer::paint_grid_fresh` emits, and the cheapest native proof that the
/// frame painted the palette that was pushed.
fn background_fill_color(ops: &[DrawOp]) -> Option<&str> {
    match ops.get(first_draw_index(ops)?) {
        Some(DrawOp::RectFill { color, .. }) => Some(color.as_str()),
        _ => None,
    }
}

/// Stage 5 (Task 1): project a frame's op stream down to the sequence of
/// `BeginGroup` classes belonging to the shared grid shell
/// (`execute_grid_shell`'s eventual contract), dropping every other op and
/// every overlay-only class. Order-preserving, so equality against
/// `[Grid, Cells, FrozenSep, Headers, Corner]` pins both membership and
/// sequence in one assertion.
fn grid_shell_group_sequence(ops: &[DrawOp]) -> Vec<GroupClass> {
    const RELEVANT: [GroupClass; 5] = [
        GroupClass::Grid,
        GroupClass::Cells,
        GroupClass::FrozenSep,
        GroupClass::Headers,
        GroupClass::Corner,
    ];
    ops.iter()
        .filter_map(|op| match op {
            DrawOp::BeginGroup { class } if RELEVANT.contains(class) => Some(*class),
            _ => None,
        })
        .collect()
}

/// Text drawn by every header cell (`draw_header_cell`) in the grid's op
/// stream, in emission order.
fn grid_fill_text_values(ops: &[DrawOp]) -> Vec<&str> {
    ops.iter()
        .filter_map(|op| match op {
            DrawOp::FillText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Default row-header labels (`PaneSet::resolve_row_labels`) are plain
/// decimal row numbers unless a model overrides them — `TestModel` never
/// does, so this distinguishes row-header text from column-header text
/// (letters) and from cell content (empty on the unpopulated `synthetic_grid`
/// fixture) without reaching into the private `Axis`/`GridHeaderScope` types.
fn is_row_label(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// Default column-header labels (`PaneSet::resolve_col_labels`) are
/// Excel-style letters (A, B, ..., AA, ...) unless a model overrides them.
fn is_col_label(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphabetic())
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

    // A content-dirty signal keeps the viewport stable -> plan_frame plans
    // SlotsReuse. CONTENT blocks the Viewport arm (blit on stale content is
    // the recalc bug), and reaching this far with no geometry/view work
    // selects SlotsReuse. Theme swaps no longer reach this regime — they
    // invalidate the paint cache and force Fresh.
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
    // bail empty — last_frame stays populated, Chrome::classify catches the
    // viewport shift as FrameDelta::Scroll, and plan_frame routes it to
    // Viewport.
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

/// Stage 5 pin (Task 1, bullets 2+3): the four grid-painting regimes must
/// share one outer scaffold — `Grid, Cells, FrozenSep, Headers, Corner`, in
/// that order, with every `BeginGroup` balanced by an `EndGroup` — before
/// `execute_grid_shell` consolidates their four independent copies of it.
/// Drives all four through one `Orchestrator` in sequence so each assertion
/// exercises the exact production dispatch path a browser would use.
#[test]
fn fresh_slots_reuse_damage_viewport_share_the_grid_shell_group_order() {
    let expected = vec![
        GroupClass::Grid,
        GroupClass::Cells,
        GroupClass::FrozenSep,
        GroupClass::Headers,
        GroupClass::Corner,
    ];
    let assert_shell = |ops: &[DrawOp], label: &str| {
        assert_eq!(
            grid_shell_group_sequence(ops),
            expected,
            "{label} must open the shared Grid/Cells/FrozenSep/Headers/Corner \
             groups in that order; got {ops:#?}"
        );
        let begins = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::BeginGroup { .. }))
            .count();
        let ends = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::EndGroup))
            .count();
        assert_eq!(
            begins, ends,
            "{label} must balance every BeginGroup with an EndGroup"
        );
    };

    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));

    let before = grid_ops_len(&orch);
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert_shell(&grid_ops_since(&orch, before), "Fresh");

    let before = grid_ops_len(&orch);
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
    assert_shell(&grid_ops_since(&orch, before), "SlotsReuse");

    let before = grid_ops_len(&orch);
    orch.mark_rows_damaged(0, RowSpan { r1: 2, r2: 2 });
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Damage));
    assert_shell(&grid_ops_since(&orch, before), "Damage");

    let before = grid_ops_len(&orch);
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));
    assert_shell(&grid_ops_since(&orch, before), "Viewport");
}

/// Stage 5 pin (Task 1, bullet 5): a row scroll's `Viewport` frame repaints
/// only the row-header strip — the plan's `GridHeaderScope::Axis(Row)` — so
/// its new ops must contain row-header content and must NOT contain any
/// column-header content, even though the `Headers` group still opens (see
/// `fresh_slots_reuse_damage_viewport_share_the_grid_shell_group_order`).
/// Asserted against the observable op stream, not the private enum.
#[test]
fn viewport_row_scroll_repaints_row_headers_but_not_column_headers() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let before = grid_ops_len(&orch);
    stub.set_top_row(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));

    let new_ops = grid_ops_since(&orch, before);
    let texts = grid_fill_text_values(&new_ops);
    assert!(
        texts.iter().any(|t| is_row_label(t)),
        "a row scroll must repaint row-header labels; got {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| is_col_label(t)),
        "a row scroll must NOT repaint column-header labels; got {texts:?}"
    );
}

/// Mirror of the row-scroll pin above, for the column axis.
#[test]
fn viewport_column_scroll_repaints_column_headers_but_not_row_headers() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh.

    let before = grid_ops_len(&orch);
    stub.set_left_column(2);
    orch.request_overlay_repaint();
    orch.paint_if_dirty();
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Viewport));

    let new_ops = grid_ops_since(&orch, before);
    let texts = grid_fill_text_values(&new_ops);
    assert!(
        texts.iter().any(|t| is_col_label(t)),
        "a column scroll must repaint column-header labels; got {texts:?}"
    );
    assert!(
        !texts.iter().any(|t| is_row_label(t)),
        "a column scroll must NOT repaint row-header labels; got {texts:?}"
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
    // unchanged -> FrameDelta::Stable, and plan_frame picks Overlay.
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
    // plan_frame picks SlotsReuse with mask = ALL.
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
/// The fix lives in `plan_frame`'s `OverlayWork` calculation: row/pane
/// content work paints the overlay when overlay work is marked, or when
/// captured selection visibility is true (content then implies an
/// active-cell repaint) — `paint_slots_reuse_regime` just reads
/// `plan.overlay` rather than re-deriving it.
#[test]
fn content_dirty_with_active_cell_repaints_overlay() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(1, 1));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty(); // Fresh — primes last_frame and overlay.

    let overlay_before = overlay_ops_len(&orch);

    // DEL on the active cell: model value gone, only CONTENT raised
    // (no OVERLAY mark). Pre-fix, gating solely on an explicit overlay
    // mark short-circuited and the overlay's stale active-cell pixels
    // stayed put.
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
/// mark geometry (plus panes(ALL) and overlay) so the next paint plans
/// `Fresh` and clears both layers — `plan_frame`'s "any geometry work
/// forces Fresh" rule guarantees this regardless of what `Chrome::classify`
/// would otherwise report (independently, the changed `model_generation`
/// also hard-breaks `Chrome::classify` to `Rebuild(Model)`). Without the
/// geometry mark, swapping the orchestrator's model in place (RustyCalc
/// workbook switch, driven by the `current_uuid` Effect in `worksheet.rs`)
/// could plan a stale SlotsReuse or Overlay paint that never repaints the
/// grid, keeping the prior workbook's chrome / pane geometry / cached pane
/// buffers on screen.
///
/// Tests the contract via the public `last_regime` accessor instead of
/// reaching into private fields: `Fresh` after `set_model` is exactly the
/// behavior the geometry mark is supposed to produce.
#[test]
fn set_model_marks_geometry_and_forces_fresh() {
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
        "set_model must mark geometry work so the next paint takes Fresh; \
         got {:?} — the workbook-switch stale-paint bug is back",
        orch.last_regime(),
    );

    let new_grid_ops = grid_ops_since(&orch, grid_before);
    let new_overlay_ops = overlay_ops_since(&orch, overlay_before);
    assert!(
        !new_grid_ops.is_empty(),
        "set_model must raise GEOMETRY so Fresh repaints the grid"
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
/// return (grid_ops, overlay_ops, regime, work_bits). The `.icr` attempt
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
    // pixels stale. `Chrome::classify` rejects the theme-mismatched frame
    // with `Rebuild(Theme)`, so the next paint takes the Fresh arm;
    // SlotsReuse would repaint stale-color cells under fresh chrome.
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    orch.set_theme(CanvasTheme::dark());
    let (grid_ops, _, regime, _) = paint_and_capture(&mut orch);

    assert_eq!(regime, Some(PaintRegimeTag::Fresh));
    assert!(!grid_ops.is_empty());
}

/// Stage 6, Gate A: `set_theme` no longer invalidates the grid painter
/// eagerly, so the healthy Fresh prologue is the *only* source of the
/// invalidate/reset pair.
///
/// The capture window opens before `set_theme`, not before `paint_if_dirty`
/// — an eager invalidation happens outside a `begin_frame`/`end_frame`
/// bracket that starts at the paint, which is exactly how the Stage 6 probe
/// first under-counted the pair.
#[test]
fn theme_change_emits_exactly_one_invalidation_pair_before_the_first_draw() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    let dark = CanvasTheme::dark();
    let dark_bg = dark.cell_bg.clone().into_owned();

    orch.grid_surface().begin_frame();
    orch.set_theme(dark);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let grid_ops = orch.grid_surface().end_frame();

    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert_eq!(
        count_op(&grid_ops, &DrawOp::InvalidateCache),
        1,
        "a theme change plus its Fresh paint must invalidate the grid painter once, not twice",
    );
    assert_eq!(
        count_op(&grid_ops, &DrawOp::ResetTextDefaults),
        1,
        "the text-default reset is paired with the invalidation and must not double either",
    );
    assert_eq!(
        first_draw_index(&grid_ops),
        Some(2),
        "the surviving pair must still lead the frame: [InvalidateCache, ResetTextDefaults, bg fill, ..]",
    );
    assert_eq!(
        background_fill_color(&grid_ops),
        Some(dark_bg.as_str()),
        "the frame's first draw must fill with the newly pushed palette",
    );
}

/// Stage 6, Gate A, second half: a theme Fresh held by a bridge failure
/// during preparation must leave the painter completely untouched — with
/// the eager call gone there is no invalidation to strand ahead of a frame
/// that never paints — and its healthy retry supplies one pair and the new
/// palette.
#[test]
fn held_theme_fresh_emits_no_painter_op_and_its_retry_emits_one_pair() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    let dark = CanvasTheme::dark();
    let dark_bg = dark.cell_bg.clone().into_owned();

    stub.set_bulk_bridge_fail(true);
    orch.grid_surface().begin_frame();
    orch.set_theme(dark);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    let held_ops = orch.grid_surface().end_frame();

    assert!(
        held_ops.is_empty(),
        "a held theme Fresh must emit no grid painter op at all, got {held_ops:?}",
    );

    stub.set_bulk_bridge_fail(false);
    orch.grid_surface().begin_frame();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let retry_ops = orch.grid_surface().end_frame();

    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::Fresh));
    assert_eq!(count_op(&retry_ops, &DrawOp::InvalidateCache), 1);
    assert_eq!(count_op(&retry_ops, &DrawOp::ResetTextDefaults), 1);
    assert_eq!(first_draw_index(&retry_ops), Some(2));
    assert_eq!(
        background_fill_color(&retry_ops),
        Some(dark_bg.as_str()),
        "the retry must paint the theme the held attempt was pushed with",
    );
}

/// Stage 6, Gate A: the `theme != *self.theme` guard still swallows an
/// equal re-push, so no invalidation, no work, no frame.
#[test]
fn reapplying_an_equal_theme_stays_idle() {
    let stub = Rc::new(TestModel::synthetic_grid());
    let mut orch = build_rec(Rc::clone(&stub));
    paint_and_capture(&mut orch); // Fresh.

    orch.grid_surface().begin_frame();
    orch.set_theme(CanvasTheme::light()); // The theme `Orchestrator::new` starts with.
    assert_eq!(orch.paint_if_dirty(), PaintResult::Idle);
    let grid_ops = orch.grid_surface().end_frame();

    assert!(
        grid_ops.is_empty(),
        "an equal theme must not reach the painter, got {grid_ops:?}",
    );
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
        let (grid_ops, overlay_ops, _, _) = paint_and_capture(orch);
        let idx = frames.len() as u32;
        let trace = TraceRecord::from(orch.last_trace());
        frames.push(Frame {
            frame_idx: idx,
            t_ms,
            origin: RecordOrigin::Live,
            result: RecordedPaintResult::Painted,
            trace,
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
    let regimes: Vec<_> = frames
        .iter()
        .map(|f| f.trace.regime.expect("regime must be stamped"))
        .collect();
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

    // Order, not just presence: cells must open (and the frozen separator's
    // pixels must win back the band's re-stroked grid lines) strictly before
    // headers/corner, matching every other grid regime's shell order.
    assert_eq!(
        grid_shell_group_sequence(&ops),
        vec![
            GroupClass::Grid,
            GroupClass::Cells,
            GroupClass::FrozenSep,
            GroupClass::Headers,
            GroupClass::Corner,
        ],
        "damage paint's chrome groups must open Grid, Cells, FrozenSep, \
         Headers, Corner in that order; got {ops:#?}"
    );
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
/// sheet, size or cell value — only the view. `Chrome::classify` reports
/// `FrameDelta::Stable` (no pixels move), and `plan_frame` must then let
/// `Overlay` match *while ignoring the view mark*.
///
/// Expressing `Overlay`'s guard as `!work.has_view()` — the mechanical port
/// of the old `!signals.grid_dirty()`, whose bit group already covered the
/// then-dead `VIEWPORT` bit — would make this row of the planner table
/// unreachable and turn the single most common interaction in the app into
/// a full-grid repaint. That regression is silent in every other test; this
/// is the one that fails.
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

/// Matching row damage can stay band-addressed when the active cell moves
/// inside the committed viewport. `FrameDelta::Stable` proves the view mark
/// did not move grid pixels, so Damage can paint the edited row and the
/// overlay can move independently.
#[test]
fn stable_row_content_plus_view_dispatches_damage() {
    let stub = Rc::new(TestModel::synthetic_grid().with_active(5, 1));
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);
    let row_rect = orch.cell_rect(5, 1).expect("edited row must be visible");
    let last_col_rect = (1..=100)
        .filter_map(|col| orch.cell_rect(5, col))
        .next_back()
        .expect("the fixture must expose at least one column");
    let expected_strip = PixelRect {
        top_left: Point {
            x: row_rect.top_left.x,
            y: row_rect.top_left.y,
        },
        width: last_col_rect.top_left.x + last_col_rect.width - row_rect.top_left.x,
        height: row_rect.height,
    };

    stub.set_cell(5, 1, "typed");
    stub.set_active(6, 1);
    orch.mark_rows_damaged(0, RowSpan { r1: 5, r2: 5 });
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let trace = orch.last_trace();
    assert_eq!(trace.regime, Some(PaintRegimeTag::Damage));
    assert_eq!(trace.effective, Some(PaintRegimeTag::Damage));
    assert_eq!(
        trace.work,
        WorkFlags::VIEW | WorkFlags::CONTENT | WorkFlags::OVERLAY
    );
    let grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. } if *rect == expected_strip)),
        "Damage must clear the exact edited-row strip {expected_strip:?}; got {grid_ops:#?}"
    );
    assert!(
        grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::FillText { text, .. } if text == "typed")),
        "the edited text must be painted inside the Damage strip"
    );
    assert!(
        !grid_ops.iter().any(|op| matches!(op, DrawOp::Blit { .. })),
        "stable content plus view must not blit"
    );
    let expected_selection = orch
        .cell_rect(6, 1)
        .expect("moved active cell must remain visible")
        .inset(1, 1);
    assert!(
        overlay_ops_since(&orch, overlay_before)
            .iter()
            .any(|op| matches!(op, DrawOp::RectStroke { rect, width, .. }
                if *rect == expected_selection && *width == 2.0)),
        "the overlay must move to the new active cell"
    );
}

#[test]
fn stable_commit_batch_selects_slots_reuse_and_moves_overlay() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen(2, 2)
            .with_active(5, 5),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);
    stub.set_cell(5, 5, "typed");
    stub.set_active(6, 5);
    orch.mark_rows_damaged(0, RowSpan { r1: 5, r2: 5 });
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let trace = orch.last_trace();
    assert_eq!(trace.regime, Some(PaintRegimeTag::SlotsReuse));
    assert_eq!(trace.effective, Some(PaintRegimeTag::SlotsReuse));
    assert_eq!(
        trace.work,
        WorkFlags::VIEW | WorkFlags::CONTENT | WorkFlags::OVERLAY
    );
    assert_eq!(
        trace.panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Rows { spans: 1, rows: 1 })
    );
    for pane in [
        PaneRegion::TopLeft,
        PaneRegion::TopRight,
        PaneRegion::BottomLeft,
    ] {
        assert_eq!(
            trace.panes[pane as usize],
            Some(PaneVerdict::Skip),
            "unchanged {pane:?} pane must fingerprint-skip"
        );
    }

    let grid_ops = grid_ops_since(&orch, grid_before);
    assert!(
        !grid_ops.iter().any(|op| matches!(op, DrawOp::Blit { .. })),
        "stable commit batch must not blit"
    );
    assert!(
        !grid_ops
            .iter()
            .any(|op| matches!(op, DrawOp::InvalidateCache)),
        "SlotsReuse must not run Fresh-only cache invalidation"
    );

    let overlay_ops = overlay_ops_since(&orch, overlay_before);
    for class in [
        GroupClass::SelectionFill,
        GroupClass::ActiveCellRepaint,
        GroupClass::SelectionStroke,
    ] {
        assert_eq!(
            overlay_ops
                .iter()
                .filter(|op| matches!(op, DrawOp::BeginGroup { class: actual } if *actual == class))
                .count(),
            1,
            "the moved selection must emit {class:?} exactly once"
        );
    }
    let expected_selection = orch
        .cell_rect(6, 5)
        .expect("moved active cell must remain visible")
        .inset(1, 1);
    assert_eq!(
        overlay_ops
            .iter()
            .filter(|op| matches!(op, DrawOp::RectStroke { rect, width, .. }
                if *rect == expected_selection && *width == 2.0))
            .count(),
        1,
        "the overlay must stroke the new active cell exactly once"
    );
}

#[test]
fn stable_commit_with_hidden_selection_uses_slots_reuse_without_active_cell_repaint() {
    let stub = Rc::new(
        TestModel::synthetic_grid()
            .with_data_until(30)
            .with_frozen(2, 2)
            .with_active(5, 5)
            .with_show_selection(false),
    );
    let mut orch = build(Rc::clone(&stub));
    orch.paint_if_dirty();

    let grid_before = grid_ops_len(&orch);
    let overlay_before = overlay_ops_len(&orch);
    let overlay_presents = orch.overlay_surface().presents();
    stub.set_cell(5, 5, "hidden-selection-edit");
    stub.set_active(6, 5);
    orch.mark_rows_damaged(0, RowSpan { r1: 5, r2: 5 });
    orch.mark_content_dirty(PaneRegionMask::ALL);
    orch.view_changed();

    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert_eq!(orch.last_regime(), Some(PaintRegimeTag::SlotsReuse));
    assert_eq!(
        orch.last_work_flags(),
        WorkFlags::VIEW | WorkFlags::CONTENT | WorkFlags::OVERLAY
    );
    assert!(
        grid_ops_since(&orch, grid_before).iter().any(
            |op| matches!(op, DrawOp::FillText { text, .. } if text == "hidden-selection-edit")
        ),
        "the committed content must reach the grid"
    );
    assert!(
        orch.overlay_surface().presents() > overlay_presents,
        "explicit view_changed overlay work must be serviced even when selection is hidden"
    );
    assert!(
        !overlay_ops_since(&orch, overlay_before)
            .iter()
            .any(|op| matches!(op, DrawOp::BeginGroup { class }
                if *class == GroupClass::ActiveCellRepaint)),
        "hidden selection must suppress the active-cell repaint hook"
    );
}
