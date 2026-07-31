//! Spec for `FrameKindTag` — every Chrome constructor sets the tag, and
//! Stage 5's regime dispatch reads it. These two specs lock down the
//! `Fresh` / `SlotsReused` constructor contracts; `Blitted` is covered
//! by the scroll-blit suite which already exercises `next_frame_with_blit`.

mod common;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath, PaneRegion, PaneRegionMask};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

/// True when some `DrawOp::FillText` in `ops` contains `needle`. Local to
/// this file's one consumer below — mirrors the same needle-search idiom
/// `held_frame.rs`'s `grid_text_ops_containing` uses at the `Orchestrator`
/// level, just against a raw `RendererCore` ops slice instead of a live
/// surface's recorder.
fn text_op_contains(ops: &[DrawOp], needle: &str) -> bool {
    ops.iter()
        .any(|op| matches!(op, DrawOp::FillText { text, .. } if text.contains(needle)))
}

#[test]
fn next_frame_emits_fresh_when_no_prev() {
    let model = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    assert_eq!(frame.kind, FrameKindTag::Fresh);
    assert!(
        !frame.kind.reuses_slots(),
        "Fresh is the one kind that does not reuse slot vecs",
    );
}

#[test]
fn from_slots_reuse_emits_slots_reused() {
    let model = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let fresh = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let reused = Chrome::next(Some(fresh), &model, &inputs, FramePath::SlotsReuse);
    assert_eq!(reused.kind, FrameKindTag::SlotsReused);
    assert!(
        reused.kind.reuses_slots(),
        "SlotsReused must report reuses_slots() so render_pane fingerprint-skips engage",
    );
}

/// Stage 3 successor to the old `slots_reuse_uses_caller_supplied_stale_panes`
/// regression pin. That test proved a `SlotsReuse` frame took its pane mask
/// from the caller's `FramePath` payload rather than silently inheriting
/// `prev.stale_panes` — guarding the scroll-to-row-78 -> DEL bug, where a
/// `SlotsReuse` chasing a `Blit` (whose `stale_panes` had been narrowed to
/// the scrolled strip) skipped the unscrolled panes on the next content
/// repaint.
///
/// Stage 3 deletes `Chrome.stale_panes` and the `FramePath::SlotsReuse`
/// payload entirely: there is no field left on `Chrome` for any frame to
/// inherit or leak a pane scope through, by construction. The equivalent —
/// and now the only possible — claim lives one level up, where the mask
/// actually travels post-Stage-3: as an explicit `render_grid` parameter.
/// This proves it directly, at the real dispatch surface, rather than via a
/// field that no longer exists: two consecutive `render_grid` calls against
/// the SAME `SlotsReused` `Chrome` value, given two disjoint explicit masks,
/// must each visit EXACTLY their own mask. A regression that reintroduced
/// any form of Chrome-carried scope would make the second call either miss
/// its own pane (inheriting the first call's narrower mask) or spuriously
/// revisit the first call's pane.
#[test]
fn consecutive_render_grid_calls_carry_independent_explicit_masks() {
    let model = TestModel::synthetic_grid()
        .with_frozen_rows(2)
        .with_frozen_cols(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Prime every pane's painted-fingerprint tree via an ordinary Fresh
    // paint, then promote — mirroring `paint_skip.rs`'s
    // `promote_to_slots_reuse` pattern — so the two calls below dispatch
    // through the mismatch branch instead of the unconditional Fresh walk.
    core.render_grid(&model, &frame, PaneRegionMask::ALL);
    frame = Chrome::next(Some(frame), &model, &inputs, FramePath::SlotsReuse);

    let top_left = PaneRegion::TopLeft
        .range(&frame)
        .expect("frozen rows+cols give TopLeft a real range");
    let bottom_right = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight always has a range");
    model.set_cell(top_left.r1, top_left.c1, "top-left-edit");
    model.set_cell(bottom_right.r1, bottom_right.c1, "bottom-right-edit");

    // First call: an explicit mask naming ONLY TopLeft — mirrors what a
    // Blit's narrow `shift_panes()` would have handed a following call.
    // Materialized into an owned `Vec` immediately (`.to_vec()`) rather than
    // held as a live `Ref` across the second `render_grid` call below —
    // `RecorderPainter::ops()` borrows a `RefCell`, and that call needs its
    // own mutable borrow to record new ops.
    let ops_before = core.painter().ops().len();
    core.render_grid(&model, &frame, PaneRegionMask::TOP_LEFT);
    let first_ops: Vec<DrawOp> = core.painter().ops()[ops_before..].to_vec();
    assert!(
        text_op_contains(&first_ops, "top-left-edit"),
        "an explicit TOP_LEFT mask must repaint its own edited cell"
    );
    assert!(
        !text_op_contains(&first_ops, "bottom-right-edit"),
        "an explicit TOP_LEFT mask must not visit BottomRight at all"
    );

    // Second call: same Chrome value, a DISJOINT explicit mask. The deleted
    // `stale_panes` field would have silently carried the FIRST call's
    // narrow TOP_LEFT scope into this one; an explicit parameter cannot.
    let ops_before = core.painter().ops().len();
    core.render_grid(&model, &frame, PaneRegionMask::BOTTOM_RIGHT);
    let second_ops: Vec<DrawOp> = core.painter().ops()[ops_before..].to_vec();
    assert!(
        text_op_contains(&second_ops, "bottom-right-edit"),
        "a later, disjoint explicit mask must still repaint its own pane — \
         proving this call's scope came from its own parameter, not from \
         whatever the previous call happened to visit"
    );
    assert!(
        !text_op_contains(&second_ops, "top-left-edit"),
        "an explicit BOTTOM_RIGHT mask must not re-visit TopLeft — Chrome \
         retains no memory of the first call's scope to leak"
    );
}

/// Documents the Stage 5 invariant: adding a `FrameKindTag` variant must
/// break the dispatch in `Orchestrator::paint_viewport_regime`. Its
/// non-exhaustive `match frame.kind` (no `_ =>` arm) enforces this at
/// compile time.
///
/// To verify locally:
/// 1. Add a fourth variant `Speculative` to `FrameKindTag` in `chrome/kind.rs`.
/// 2. `cargo check -p iron-canvas-core` — expect `error[E0004]` in
///    `orchestrator.rs`.
/// 3. Revert.
///
/// This test does NOT do the experiment automatically; it pins the variant
/// list so a future reader can run the experiment in <1 minute.
#[test]
fn frame_kind_variants_documented() {
    let variants = [
        FrameKindTag::Fresh,
        FrameKindTag::SlotsReused,
        FrameKindTag::Blitted,
    ];
    assert_eq!(variants.len(), 3);
}
