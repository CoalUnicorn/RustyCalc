//! `render_pane` live-dispatch and pane-lifecycle integration tests for the
//! pane->row fingerprint repaint system.
//!
//! Pure planner/tree tests against `plan_pane_repaint` /
//! `build_pane_fingerprint` live in `fingerprint.rs`'s own `#[cfg(test)] mod
//! tests` — those types are `pub(crate)`, so only visible there. This file
//! covers what can only be observed through PUBLIC behavior:
//! `RendererCore::render_pane` / `render_pane_damage`'s actual paint
//! dispatch, read back via `RecorderPainter`'s `DrawOp` log and `PaneCache`'s
//! own public buffer-range accessor — the same recorder-based style
//! `tests/paint_skip.rs` uses.

mod common;

use iron_canvas_core::chrome::{
    ActiveCellSnapshot, BlitOutcome, BlitPlan, Chrome, FrameKindTag, FramePath, PaneRegion,
    PaneRegionMask,
};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, FrameDelta, FrameInputs, PaneVerdict, RCRange, RowSpan};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.kind = FrameKindTag::SlotsReused;
}

/// Same qualify-then-blit pair `scroll_blit.rs` drives its own fixtures with
/// (each integration test file owns its copy, as `blit_fallback.rs` and
/// `blit_golden.rs` already do) — needed here because Damage's effect on
/// fingerprint *truth* is only observable through a later shift's eligibility.
fn qualify_scroll(prev: &Chrome, m: &TestModel, inputs: &FrameInputs) -> BlitPlan {
    let view = m.get_selected_view().expect("scroll model has view");
    let snap = ActiveCellSnapshot::capture(m, view.sheet, view.row, view.column);
    match Chrome::classify(Some(prev), m, inputs, Some(&snap)) {
        FrameDelta::Scroll(plan) => plan,
        _ => panic!("row scroll must qualify for blit"),
    }
}

#[test]
fn render_pane_repaint_dispatches_a_row_span_plan_to_a_scoped_band_paint() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let changed_row = pane_range.r1 + 2;
    assert!(
        changed_row < pane_range.r2,
        "fixture needs a genuine interior row"
    );
    m.set_cell(changed_row, pane_range.c1, "changed");

    let band_rect = frame
        .range_rect(RCRange {
            r1: changed_row,
            c1: pane_range.c1,
            r2: changed_row,
            c2: pane_range.c2,
        })
        .expect("changed row's band must be visible");
    let pane_rect = frame.range_rect(pane_range).expect("pane rect visible");

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let new_ops = &ops[ops_before..];

    assert!(!new_ops.is_empty(), "a changed row must repaint");
    // Distinguishes the `Rows` path from `Full`: `Full` always opens with a
    // clear matching the *entire* pane; a scoped row-band repaint never
    // emits that op at all — every fill stays inside the one changed band.
    assert!(
        !new_ops
            .iter()
            .any(|op| matches!(op, DrawOp::RectFill { rect, .. } if *rect == pane_rect)),
        "a single safe row change must not fall back to a whole-pane clear"
    );
    assert!(
        new_ops.iter().all(|op| match op {
            DrawOp::RectFill { rect, .. } =>
                rect.top_left.y >= band_rect.top_left.y
                    && rect.top_left.y + rect.height <= band_rect.top_left.y + band_rect.height,
            _ => true,
        }),
        "render_pane must dispatch the Rows plan to a paint scoped to that row's band"
    );
}

#[test]
fn render_pane_repaint_full_and_skip_paths_are_unchanged_by_the_new_dispatch() {
    // Regression guard: the mismatch branch dispatching through
    // `plan_pane_repaint` must not alter the two paths that predate its
    // wiring — `Skip` (identical content) and the `Fresh`-frame first paint
    // (never row-banded).
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Fresh-frame first paint: always the unconditional full walk, on a
    // `!reuses_slots` frame `plan_pane_repaint` must never even run.
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(
        !core.painter().ops().is_empty(),
        "first paint of a non-empty pane must still emit ops"
    );

    promote_to_slots_reuse(&mut frame);

    // Identical content under SlotsReused: still a clean Skip, zero new ops.
    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "an idempotent repaint must still skip entirely — unaffected by the Rows dispatch"
    );
}

// ==============================================================================
// Pane lifecycle: buffer-range invalidation (`PaneCache::invalidate`) vs.
// painted-pixel invalidation (`PaneFingerprintState`), and the atomicity of
// `render_pane_strip` (the shared helper behind both the Damage and blit
// strip paths) across its four bulk buffers.
// ==============================================================================

/// `PaneCache::invalidate` (buffer-range only) must not force a repaint when
/// the refetched content is byte-identical to what was last painted — the
/// retained `painted` tree is what makes that Skip possible. This is the
/// direct behavioral proof that buffer-range staleness and painted-pixel
/// staleness are genuinely two separate axes, not just two names for the
/// same effect.
#[test]
fn lifecycle_buffer_range_invalidate_keeps_painted_tree() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame); // primes range + painted tree
    promote_to_slots_reuse(&mut frame);
    let range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");

    // Mirrors what a content-dirty SlotsReuse regime does: only the buffer
    // range is dropped, forcing a refetch next call.
    core.pane_cache.invalidate(PaneRegionMask::ALL);
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        None,
        "PaneCache::invalidate must clear the cached buffer range"
    );

    // No content changed since the prime — a retained painted tree must
    // still Skip, even though the buffer range needs a fresh fetch.
    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "buffer-range invalidation alone must not force a repaint — the \
         painted-pixel tree must survive PaneCache::invalidate"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        Some(range),
        "the Skip path must still re-park the freshly refetched buffers/range"
    );
}

/// A transient `BridgeFailed` on any one of a Damage strip's four bulk
/// buffers must reject the whole strip update atomically — no splice, no
/// clear, no paint — leaving the pane's cached buffer range and painted tree
/// exactly as they were. `TestModel::set_value_bridge_fail` fails only
/// `get_formatted_cell_value` (the *values* buffer), leaving
/// styles/cell_types/decorations to fetch successfully — a genuine
/// mid-strip-fetch partial failure, not a hypothetical all-four failure.
#[test]
fn lifecycle_bridge_failed_damage_strip_is_atomic() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);
    let range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let span = RowSpan {
        r1: range.r1 + 2,
        r2: range.r1 + 2,
    };

    m.set_value_bridge_fail(true);
    let ops_before = core.painter().ops().len();
    core.render_pane_damage(&m, &frame, PaneRegion::BottomRight, &[span]);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "a BridgeFailed strip fetch must emit no pixel operations at all"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        Some(range),
        "a failed strip fetch must leave the cached buffer range untouched"
    );

    // Restore the bridge; content is unchanged from the original prime. If
    // the failed strip left the painted tree intact (as it must), this next
    // call finds identical content and Skips.
    m.set_value_bridge_fail(false);
    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "the painted tree must survive a failed Damage strip fetch — an \
         unchanged repaint afterward must still Skip"
    );
}

/// A Damage span that never intersects a pane's own row range must not touch
/// that pane at all (its per-span loop simply never calls the strip
/// machinery) — proving Damage invalidation is scoped to intersected panes
/// only. On the pane that IS intersected, the successful strip splice
/// changes its buffers, so the very next paint must find a real mismatch
/// (not spuriously Skip) and repaint for real — reseeding a fresh, valid
/// tree that the paint AFTER that can then Skip against.
#[test]
fn lifecycle_damage_strip_scopes_to_intersected_pane_and_reseeds_on_next_paint() {
    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::TopRight, &frame);
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let top_range = PaneRegion::TopRight
        .range(&frame)
        .expect("TopRight must have a range with frozen rows");
    let bottom_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range");
    assert!(
        bottom_range.r1 > top_range.r2,
        "fixture needs TopRight and BottomRight to cover disjoint rows"
    );

    let changed_row = bottom_range.r1 + 1;
    m.set_cell(changed_row, bottom_range.c1, "changed");
    let span = RowSpan {
        r1: changed_row,
        r2: changed_row,
    };

    // TopRight: the span never intersects its range — must be untouched.
    let top_ops_before = core.painter().ops().len();
    core.render_pane_damage(&m, &frame, PaneRegion::TopRight, &[span]);
    assert_eq!(
        core.painter().ops().len(),
        top_ops_before,
        "a Damage span outside TopRight's range must not touch it at all"
    );
    core.render_pane(&m, PaneRegion::TopRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        top_ops_before,
        "TopRight's tree must still Skip — a non-intersecting Damage span \
         must never have touched it"
    );

    // BottomRight: the span DOES intersect — the strip must repaint it.
    let bottom_ops_before = core.painter().ops().len();
    core.render_pane_damage(&m, &frame, PaneRegion::BottomRight, &[span]);
    assert!(
        core.painter().ops().len() > bottom_ops_before,
        "an intersecting Damage span must repaint BottomRight's strip"
    );

    // Reseed: the strip changed BottomRight's buffers, so the very next
    // paint must find a real mismatch and repaint for real.
    let reseed_ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(
        core.painter().ops().len() > reseed_ops_before,
        "the first paint after a successful strip must reseed the tree \
         with a real repaint, not spuriously Skip"
    );

    // ...and the paint after THAT must Skip, proving the reseed committed a
    // valid, matching tree.
    let idempotent_ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        idempotent_ops_before,
        "once reseeded, an unchanged repaint must Skip again"
    );
}

/// Stage 6 Task 6: a Damage strip must mark the pane's fingerprint history
/// stale, so a row blit that follows it immediately cannot carry that history
/// forward — even though every geometric precondition for rotation is met.
///
/// This is the case range equality alone cannot catch, and the whole reason a
/// separate truth state exists: Damage repaints a band *without* changing the
/// pane's range, so the retained tree still describes `prev_range` exactly
/// when the next shift asks. `scroll_blit.rs`'s
/// `frame_trace_names_the_post_blit_slots_reuse_paint_as_skip` runs the same
/// scroll on an undamaged pane and gets `Skip`; the only difference here is
/// the Damage strip in between, and the verdict must fall back to the
/// conservative whole-pane repaint that reseeds a proven tree.
///
/// Damage never installs a tree of its own, however truthful its band looks:
/// removing a medium or thick border can leave bleed *outside* the cleared
/// band, which no candidate built from the band's cells would describe.
#[test]
fn damage_marks_history_stale_so_the_next_row_blit_cannot_rotate() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_grid(&m, &frame0, PaneRegionMask::ALL);
    promote_to_slots_reuse(&mut frame0);

    let range = PaneRegion::BottomRight
        .range(&frame0)
        .expect("BottomRight must have a range on this canvas");
    let damaged_row = range.r1 + 1;
    m.set_cell(damaged_row, range.c1, "damaged");
    core.render_pane_damage(
        &m,
        &frame0,
        PaneRegion::BottomRight,
        &[RowSpan {
            r1: damaged_row,
            r2: damaged_row,
        }],
    );

    // A row scroll small enough that the damaged row survives into the new
    // range — so the tree the rotation would have carried forward is exactly
    // the one the Damage band invalidated.
    m.set_top_row(range.r1 + 1);
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs);
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let BlitOutcome::Blitted(mut frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan)
    else {
        panic!("row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);
    promote_to_slots_reuse(&mut frame1);

    core.reset_trace();
    core.render_pane(&m, PaneRegion::BottomRight, &frame1);
    assert_eq!(
        core.trace().panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Full),
        "a row blit over Damage-stale history must not rotate — the next paint \
         reseeds the whole pane"
    );
}

// ==============================================================================
// Stage 4 pin (Task 1, bullet 4): SlotsReuse's partial commit, expressed
// directly in `PaneCache` terms rather than the model's bulk-fetch-count
// proxy `held_frame.rs`'s Orchestrator-level
// `partial_commit_paints_healthy_panes_and_retries_held_panes` already uses.
// GREEN today — Stage 3 already isolates a pane's cache invalidation from
// its siblings correctly; this is the property Stage 4 must keep.
// ==============================================================================

#[test]
fn slots_reuse_partial_commit_preserves_healthy_pane_range_and_narrows_retry_to_failed_pane() {
    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_grid(&m, &frame, PaneRegionMask::ALL); // primes TopRight + BottomRight.
    let healthy_range = core.pane_cache.pane(PaneRegion::TopRight).range.get();
    assert!(
        healthy_range.is_some(),
        "TopRight must be primed by the Fresh paint"
    );
    // `render_pane`'s bridge-failure hold branch only fires under
    // `frame.kind.reuses_slots()` — a Fresh-kind frame always takes the
    // unconditional full repaint (see its own doc comment). Promote to
    // `SlotsReused` so the second `render_grid` call below actually
    // exercises the SlotsReuse content-dirty dispatch this test targets,
    // exactly as `paint_slots_reuse_regime` builds its own candidate via
    // `Chrome::next(.., FramePath::SlotsReuse)`.
    promote_to_slots_reuse(&mut frame);

    m.set_cell(1, 1, "top-edit");
    m.set_cell(6, 1, "bottom-edit");
    m.set_bulk_bridge_fail_from(Some(3)); // fails BottomRight's fetch only.

    // Mirrors `paint_slots_reuse_regime`'s own
    // `invalidate_pane_cache(mask)` + `paint_grid(.., mask)` sequence.
    core.pane_cache.invalidate(PaneRegionMask::ALL);
    let held = core.render_grid(&m, &frame, PaneRegionMask::ALL);

    assert_eq!(
        held,
        PaneRegionMask::BOTTOM_RIGHT,
        "only the scroll-band pane's fetch must fail"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::TopRight).range.get(),
        healthy_range,
        "the healthy pane's cached range must be untouched by the sibling's failure"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        None,
        "the failed pane's range stays invalidated until its own retry succeeds"
    );

    // Retry: mirrors `Orchestrator::requeue_held_panes` — only the held
    // mask is invalidated and repainted, never the healthy sibling.
    m.set_bulk_bridge_fail_from(None);
    core.pane_cache.invalidate(held);
    let held_again = core.render_grid(&m, &frame, held);

    assert_eq!(
        held_again,
        PaneRegionMask::EMPTY,
        "the narrowed retry must succeed"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::TopRight).range.get(),
        healthy_range,
        "the narrowed retry must never have touched the healthy pane's range"
    );
    assert!(
        core.pane_cache
            .pane(PaneRegion::BottomRight)
            .range
            .get()
            .is_some(),
        "the retried pane must be freshly re-cached after its recovery"
    );
}
