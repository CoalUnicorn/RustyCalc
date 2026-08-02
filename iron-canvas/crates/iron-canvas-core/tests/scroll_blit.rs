//! Stage 2.4 — recorder-driven proof that the scroll-blit fast-path
//! activates when it should and stays disabled when it shouldn't.
//!
//! These tests drive `RendererCore` directly (one `RecorderPainter`,
//! one `RendererCore` across both frames) so the cross-frame pane cache
//! survives between paints — that's the state the Stage 3.3a strip-fetch
//! path depends on.

mod common;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::CanvasSize;
use iron_canvas_core::RowSpan;
use iron_canvas_core::chrome::{
    ActiveCellSnapshot, BlitOutcome, BlitPlan, Chrome, FrameKindTag, FramePath, PaneRegion,
    PaneRegionMask,
};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    BlitPaneWork, FrameDelta, FrameInputs, PaneBlitAddressWork, PaneShiftPrep, PaneVerdict,
    RebuildReason, widen_blit_strip_to_pixel_clip,
};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default as canvas, test_inputs};

/// Capture an `ActiveCellSnapshot` from the model's view. `Chrome::classify`
/// takes the snapshot as a parameter; tests source it here instead of
/// reading it off `Chrome`.
fn snap(m: &TestModel) -> ActiveCellSnapshot {
    let view = m.get_selected_view().expect("scroll model has view");
    ActiveCellSnapshot::capture(m, view.sheet, view.row, view.column)
}

/// Classify `prev` against `m`'s live state via the captured `inputs` and
/// `m`'s current active-cell snapshot, panicking with `msg` unless the
/// delta actually qualifies for a scroll-blit. Every test below drives the
/// same qualify-then-blit sequence `Chrome::classify` (qualification) +
/// `Chrome::next_blit` (construction) replaced `screen_for_blit` +
/// `next_blit` with.
fn qualify_scroll(
    prev: &Chrome,
    m: &TestModel,
    inputs: &FrameInputs,
    msg: &'static str,
) -> BlitPlan {
    match Chrome::classify(Some(prev), m, inputs, Some(&snap(m))) {
        FrameDelta::Scroll(plan) => plan,
        _ => panic!("{msg}"),
    }
}

fn count_blits(ops: &[DrawOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DrawOp::Blit { .. }))
        .count()
}

fn count_rect_fills(ops: &[DrawOp]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, DrawOp::RectFill { .. }))
        .count()
}

#[test]
fn scroll_by_one_row_emits_exactly_one_blit_op() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    // Frame 0 at top_row=1.
    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    let baseline_ops = core.painter().ops().len();

    // Scroll by 1 row -> top_row=2.
    m.set_top_row(2);

    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-row scroll must qualify for blit",
    );

    // Simulate the orchestrator's blit fast-path on the same core so
    // the pane cache state carries across frames.
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    assert_eq!(
        count_blits(&blit_phase_ops),
        1,
        "blit fast-path must emit exactly one DrawOp::Blit, got {:#?}",
        blit_phase_ops,
    );

    // The recorded Blit must match the plan's src/dst exactly — the
    // painter shouldn't be reinterpreting coordinates.
    let blit_op = blit_phase_ops
        .iter()
        .find(|op| matches!(op, DrawOp::Blit { .. }))
        .expect("blit op present per earlier assertion");
    let primary = match plan.shifts.first() {
        Some(s) => s,
        None => panic!("plan must carry at least one shift"),
    };
    match blit_op {
        DrawOp::Blit { src, dst } => {
            assert_eq!(*src, primary.src, "blit src must match plan");
            assert_eq!(*dst, primary.dst, "blit dst must match plan");
        }
        _ => unreachable!(),
    }
}

#[test]
fn scroll_past_viewport_disqualifies_blit() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);

    // Canvas is 400 px tall, rows are 20 px -> ~20 visible rows. Scroll
    // by 100 rows -> no overlap with prev viewport -> classify must reject it.
    m.set_top_row(101);

    let inputs = test_inputs(&m, canvas, &theme);
    let delta = Chrome::classify(Some(&frame0), &m, &inputs, Some(&snap(&m)));
    assert!(
        matches!(
            delta,
            FrameDelta::Rebuild(RebuildReason::IncompatibleScrollOverlap)
        ),
        "scroll past viewport extent must not qualify for blit",
    );
}

#[test]
fn scroll_by_one_column_emits_exactly_one_blit_op() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);
    let baseline_ops = core.painter().ops().len();

    // Pure horizontal scroll by 1 column.
    m.set_left_column(2);

    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-column scroll must qualify for blit",
    );

    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    assert_eq!(
        count_blits(&blit_phase_ops),
        1,
        "column-scroll blit fast-path must emit exactly one DrawOp::Blit, got {:#?}",
        blit_phase_ops,
    );
}

/// Regression for the strip-fetch path: a 1-row scroll must paint only
/// the freshly-revealed strip, not the kept band. `apply_blit_shift`
/// rotates kept-band entries into their new pane indices (still `Some`),
/// so an unqualified full-pane walk would re-take them and emit cell-bg
/// rect_fills on top of pixels the painter blit already placed. The fix
/// narrows iteration to the strip via `PaneCells::for_strip`; this test
/// locks that contract in by comparing post-blit cell paint volume
/// against the full-pane baseline.
#[test]
fn scroll_by_one_row_paints_only_strip_cells() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    let baseline_ops: Vec<DrawOp> = core.painter().ops().iter().cloned().collect();
    let baseline_rect_fills = count_rect_fills(&baseline_ops);

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-row scroll must qualify for blit",
    );
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    let blit_phase_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops.len())
        .cloned()
        .collect();
    let blit_phase_rect_fills = count_rect_fills(&blit_phase_ops);

    // Strip = 2 rows of cells (prev's overflow row + new's overflow row,
    // see `compute_strip` for why both) + 1 strip-bg fill + the row-header
    // strip (scroll-axis header always repaints) + corner box. Full-pane
    // repaint is O(visible_rows × visible_cols). With ~19 rows × 7 cols
    // visible, a buggy strip path that walks the full pane emits roughly
    // the same cell rect_fills as the baseline; the strip-only path emits
    // a small constant + headers. `×3 <` keeps catching a kept-band leak
    // (which would push past the full baseline) while tolerating the
    // 2-row strip shape.
    assert!(
        blit_phase_rect_fills * 3 < baseline_rect_fills,
        "1-row strip path emitted {} rect_fills; full-pane baseline was {}. \
         the strip path must not re-paint the kept band",
        blit_phase_rect_fills,
        baseline_rect_fills,
    );
}

/// Regression for the edit-then-scroll bug: typing into a cell and
/// pressing Enter scrolls by one row. If the consumer forgets to call
/// `markContentDirty`, the CONTENT-veto in `decide()` doesn't fire, so
/// the geometric scroll-origin fork in `Chrome::classify` would otherwise
/// succeed and the blit's kept band would shift pre-edit pixels (the
/// just-edited cell renders blank). The defensive content check inside
/// `Chrome::classify` must catch this by re-hashing the prev frame's
/// active-cell value against the live model.
#[test]
fn active_cell_value_change_disqualifies_blit() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();
    // Frame 0 paints with row 1 ("R1" coords) returning "".
    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    // Snapshot captured at paint time, as SelectionLayer would. The
    // defensive check in `Chrome::classify` compares this snapshot
    // against the live model on the *next* blit attempt.
    let active_at_paint = snap(&m);

    // Simulate the bug: row 1's value flips "" -> "R1" (proxy for an edit
    // committed at the active cell) AND viewport scrolls by one row.
    m.set_data_until(5);
    m.set_top_row(2);

    let inputs = test_inputs(&m, canvas, &theme);
    let delta = Chrome::classify(Some(&frame0), &m, &inputs, Some(&active_at_paint));
    assert!(
        matches!(
            delta,
            FrameDelta::Rebuild(RebuildReason::ActiveCellChangedOrUnknown)
        ),
        "edit-then-scroll must disqualify the blit when the active cell value changed",
    );
}

/// Contrapositive: a pure single-row scroll with the active-cell value
/// unchanged still qualifies for the blit fast path. Pins the defensive
/// check to mismatch-only behavior — it must not over-reject.
#[test]
fn active_cell_value_unchanged_allows_blit() {
    let m = TestModel::synthetic_grid();
    m.set_data_until(20); // row 1's value is "R1" in both frames.
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();
    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);

    m.set_top_row(2);

    let inputs = test_inputs(&m, canvas, &theme);
    let delta = Chrome::classify(Some(&frame0), &m, &inputs, Some(&snap(&m)));
    assert!(
        matches!(delta, FrameDelta::Scroll(_)),
        "pure scroll with unchanged active-cell value must qualify for the blit",
    );
}

/// Task 5, acceptance criterion 4 (row-axis half): `render_grid_blit` only
/// visits `plan.shift_panes()`, so a pure row scroll must never touch the
/// frozen row band's panes at all — proven directly via the public
/// `PaneBuffers::range` field staying byte-identical to what the priming
/// Fresh paint set, not just "no ops observed."
#[test]
fn row_scroll_leaves_frozen_row_band_panes_untouched() {
    // Frozen rows only (mirrors `col_scroll_with_frozen_rows_includes_top_right_work`'s
    // fixture): TopRight exists as the frozen row band; TopLeft/BottomLeft don't
    // (no frozen columns), so only TopRight is meaningful to check here.
    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    let top_right_before = core.pane_cache.pane(PaneRegion::TopRight).range.get();
    assert!(
        top_right_before.is_some(),
        "TopRight must be primed by Fresh"
    );

    // With frozen_rows=2, `scroll_first` clamps the scroll band's first row
    // to `max(frozen+1, view.top_row)` — top_row=1 or 2 both clamp to 3
    // (frame0's own starting point), which would be a no-op scroll. top_row=4
    // is the first value that actually shifts the scroll band.
    m.set_top_row(4);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "row scroll must qualify for blit");
    assert!(plan.shift_panes().contains_region(PaneRegion::BottomRight));
    assert!(
        !plan.shift_panes().contains_region(PaneRegion::TopRight),
        "a row-axis scroll must not shift the frozen row band"
    );

    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    assert_eq!(
        core.pane_cache.pane(PaneRegion::TopRight).range.get(),
        top_right_before,
        "row scroll must not touch the frozen row band's cache at all"
    );
    // The shifted pane's cache DOES change (rotated to the new range) —
    // the asymmetry is the point of this test.
    assert_ne!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        None
    );
}

/// Task 5, acceptance criterion 4 (column-axis half): mirror of the row-axis
/// test above — a pure column scroll must leave the frozen column band
/// (`BottomLeft`) untouched.
#[test]
fn column_scroll_leaves_frozen_column_band_panes_untouched() {
    // Frozen columns only (mirrors `row_scroll_with_frozen_cols_includes_bottom_left_work`'s
    // fixture): BottomLeft exists as the frozen column band.
    let m = TestModel::synthetic_grid().with_frozen_cols(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    let bottom_left_before = core.pane_cache.pane(PaneRegion::BottomLeft).range.get();
    assert!(
        bottom_left_before.is_some(),
        "BottomLeft must be primed by Fresh"
    );

    // Mirror of the row-axis test's `scroll_first` clamping note, on columns.
    m.set_left_column(4);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "column scroll must qualify for blit");
    assert!(plan.shift_panes().contains_region(PaneRegion::BottomRight));
    assert!(
        !plan.shift_panes().contains_region(PaneRegion::BottomLeft),
        "a column-axis scroll must not shift the frozen column band"
    );

    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("column scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomLeft).range.get(),
        bottom_left_before,
        "column scroll must not touch the frozen column band's cache at all"
    );
    assert_ne!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        None
    );
}

/// Task 5, acceptance criterion 5, via the blit path specifically. The only
/// existing reseed test (`row_fingerprint_repaint.rs`'s
/// `lifecycle_damage_strip_scopes_to_intersected_pane_and_reseeds_on_next_paint`)
/// drives `render_pane_damage`; `render_pane_blit` shares `render_pane_
/// strip`'s body but that sharing was inferred, not demonstrated, for the
/// blit caller. Proven here directly: after a row scroll shifts
/// `BottomRight` (invalidating its painted tree via the strip splice), the
/// very next paint with unchanged content must find a real mismatch and
/// reseed a fresh tree — and the paint after THAT must Skip, proving the
/// reseed actually committed.
#[test]
fn row_scroll_shifted_pane_reseeds_and_skips_on_next_unchanged_paint() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "row scroll must qualify for blit");
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(mut frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan)
    else {
        panic!("row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    // A strip splice never commits into the painted tree — the scroll
    // changed the pane's live range, and range is baked into the digest, so
    // the reseed below is forced by the range mismatch itself. The
    // reseed→repaint→skip sequence proves that end to end through public
    // draw-op behaviour.

    // Promote to a plain SlotsReuse frame at the SAME geometry (content
    // unchanged since the blit) to drive `render_pane`'s own mismatch/skip
    // dispatch directly, mirroring `row_fingerprint_repaint.rs`'s
    // `promote_to_slots_reuse` helper.
    frame1.kind = FrameKindTag::SlotsReused;

    let reseed_ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame1);
    assert!(
        core.painter().ops().len() > reseed_ops_before,
        "the first paint after a shifted pane's strip must reseed the tree \
         with a real repaint, not spuriously Skip"
    );

    let idempotent_ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame1);
    assert_eq!(
        core.painter().ops().len(),
        idempotent_ops_before,
        "once reseeded via the blit path, an unchanged repaint must Skip again"
    );
}

/// Stage 1 of `docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`,
/// in machine-checkable form: the design's whole premise is that the first
/// `SlotsReuse` paint after a blit reports `Full` even though nothing changed,
/// because the strip path never committed the tree and `plan_pane_repaint`
/// treats the resulting range mismatch as unconditionally `Full`.
///
/// The sibling test above proves the same thing through draw-op counts. This
/// one names it, so if a later change makes the post-blit paint cheap, the
/// trace says which verdict replaced `Full` instead of just "fewer ops".
#[test]
fn frame_trace_names_the_post_blit_slots_reuse_paint_as_full() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "row scroll must qualify for blit");
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(mut frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan)
    else {
        panic!("row scroll must blit in place");
    };

    core.reset_trace();
    core.render_grid_blit(&m, &frame1, &plan);
    let blit_trace = core.trace();
    assert_eq!(
        blit_trace.panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Strip),
        "the blit frame itself must report the cheap strip path"
    );
    assert!(
        blit_trace.fetched_cell_slots > 0,
        "a strip fetch is still four bulk accessor calls"
    );

    frame1.kind = FrameKindTag::SlotsReused;

    core.reset_trace();
    core.render_pane(&m, PaneRegion::BottomRight, &frame1);
    assert_eq!(
        core.trace().panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Full),
        "the first post-blit SlotsReuse paint repaints the whole pane despite \
         unchanged content — the spike this design targets"
    );

    core.reset_trace();
    core.render_pane(&m, PaneRegion::BottomRight, &frame1);
    assert_eq!(
        core.trace().panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Skip),
        "once reseeded, an unchanged repaint skips"
    );
    assert!(
        core.trace().fetched_cell_slots > 0,
        "invariant I1: even a Skip pays the full four-accessor round-trip"
    );
}

/// The 55 ms browser spike, reduced to a fixture. A pane that reaches a blit
/// frame without a usable cached range cannot be strip-painted, so
/// `render_grid_blit` hands it to the full `render_pane` — after
/// `unshiftable_pane_is_safe` has already fetched and bridge-validated that
/// same full range. Fetching it twice is the cost that made the frame
/// pathological; the fallback must adopt the preflight's buffers instead.
///
/// Asserted through `FrameTrace.fetched_cell_slots` rather than a cell count so
/// the test states the invariant ("no second round-trip") rather than a
/// viewport-dependent number.
#[test]
fn unshiftable_pane_on_a_blit_frame_fetches_once_not_twice() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "row scroll must qualify for blit");
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("row scroll must blit in place");
    };

    // Force every pane off the strip path. A cold cache is the reproducible
    // half of the browser case; the other half (`IncompatibleRange`, a visible
    // row count that changed by one) reaches the same fallback.
    core.pane_cache.invalidate(PaneRegionMask::ALL);

    core.reset_trace();
    let held = core.render_grid_blit(&m, &frame1, &plan);
    assert!(!held, "a healthy bridge must not abort the frame");

    // `prepare_blit`'s fallback arm calls `prepare_full_pane` exactly once
    // for this pane — a single bulk-fetch round over its whole range, never a
    // safety fetch followed by a second `render_pane`-style refetch.
    let range = PaneRegion::BottomRight
        .range(&frame1)
        .expect("BottomRight has a live range on this canvas");
    let expected_single_fetch = range.height() as usize * range.width() as usize * 4;

    let trace = core.trace();
    assert_eq!(
        trace.fetched_cell_slots, expected_single_fetch,
        "the unshiftable pane's full-range fallback must fetch exactly once, \
         not cross the bridge a second time for the same cells"
    );
    assert_eq!(
        trace.panes[PaneRegion::BottomRight as usize],
        Some(PaneVerdict::Full),
        "an unshiftable pane still repaints in full — only the refetch is gone"
    );
    assert!(
        trace
            .blit_fallback
            .is_some_and(|fb| fb.pane == PaneRegion::BottomRight && fb.cold_cache),
        "the trace must name the pane that lost the strip path, and why"
    );
}

/// SESSION.md 2026-07-24's missing fixture: `unshiftable_pane_is_safe`
/// (`renderer/cell/mod.rs:648`) bridge-validates a pane's full range before
/// letting the frame proceed when that pane couldn't stage a strip — but no
/// test exercised the FAILING half of that validation. Distinct from
/// `unshiftable_pane_on_a_blit_frame_fetches_once_not_twice` above (a healthy
/// cold-cache demotion, already pinned): this is the same cold-cache
/// classification with a bridge failure on the pane's own validating fetch,
/// which must hold the WHOLE frame atomically (no shift, no paint) — mirrors
/// `blit_preflight_bridge_failure_aborts_frame_without_shifting`'s contract,
/// but for the cold-cache door rather than the revealed-strip door.
#[test]
fn cold_cache_bridge_failure_holds_the_whole_blit_frame() {
    let m = TestModel::synthetic_grid();
    m.set_data_until(30); // Real content, so a stray paint would be visible.
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-row scroll must qualify for blit",
    );
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };

    // Cold cache: force BottomRight into `MissingCache` instead of `Shifted`,
    // so the preflight routes it through `unshiftable_pane_is_safe` rather
    // than the strip path.
    core.pane_cache.invalidate(PaneRegionMask::ALL);
    let range_before = core.pane_cache.pane(PaneRegion::BottomRight).range.get();
    assert_eq!(range_before, None, "invalidate must clear the cached range");

    // Bulk knob, not the values-only flag: `unshiftable_pane_is_safe` fetches
    // all four accessors, and any one BridgeFailed must fail the validation.
    m.set_bulk_bridge_fail(true);

    let baseline_ops = core.painter().ops().len();

    let held = core.render_grid_blit(&m, &frame1, &plan);

    assert!(
        held,
        "a failing validation fetch on a cold-cache (unshiftable) pane must \
         abort the whole frame — the second door of preparation"
    );

    let new_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();
    assert!(
        new_ops.is_empty(),
        "an aborted blit frame must be a complete no-op for the grid layer, got: {new_ops:#?}"
    );
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        range_before,
        "an aborted blit frame must leave the pane's cached range exactly as it was"
    );
}

/// Review finding 7's other uncovered door: a pane demoted for
/// `IncompatibleRange` (SESSION.md 2026-07-25's 55 ms scroll spike —
/// `shift_is_safe` rejects a row scroll whose visible row count changed)
/// must repaint `Full`, never fingerprint-`Skip`/`Rows` over pixels that
/// were never actually blitted into place.
///
/// Geometry: `Chrome::classify`/`next_blit` gate on `canvas == self.canvas_size`
/// (`chrome/mod.rs:431`), and `rebuild_axis_slots`'s trim/top-up contract
/// (`chrome/blit_rebuild.rs`) provably preserves row COUNT across any single
/// in-bounds shift as long as the SAME canvas height is used throughout: the
/// scroll band starts at `origin_y = HEADER_ROW_HEIGHT + CELL_AREA_INSET`
/// (29 px, `chrome/mod.rs` Phase B) and `fill_axis` always ends on the first
/// slot whose start reaches `max_cursor` (the canvas height), so a 1-row
/// forward scroll always drops exactly one leading slot and tops up exactly
/// one trailing slot — verified both by hand and empirically (a
/// `temp_negative_control` variant of this test, run once during
/// development and removed, held the SAME canvas throughout and landed on
/// the ordinary `Strip` verdict with `blit_fallback: None`). So a stable
/// canvas can't reach `IncompatibleRange` this way; the real trigger is the
/// one `pane_cache.rs`'s own doc comment on `PaneShiftPrep` already names:
/// "a frame before a canvas resize". Reproduced directly as that: the pane
/// cache is seeded by an earlier `render_grid` at (600, 590) — 20 px rows
/// don't divide `590 - 29 = 561` evenly (28.05 rows), so `fill_axis`'s
/// ceiling rule lands on row 29 as the last (1 px) partially-visible row and
/// row 30 as the fully off-canvas overflow slot, giving `BottomRight` the
/// range (1, 30) — while the scroll/blit sequence under test runs entirely
/// at the OTHER, internally self-consistent canvas (600, 400), giving
/// `BottomRight` the range (2, 21) after the 1-row scroll. (1,30) spans 29
/// rows; (2,21) spans 19 — the mismatched counts are exactly the
/// stale-cache-across-a-resize case `shift_is_safe` exists to catch. Neither
/// half is geometrically wrong on its own; both were confirmed via the
/// `PaneRegion::range` values printed during development.
#[test]
fn incompatible_range_demotion_repaints_full_not_skip_or_rows() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Seed the pane cache from a canvas height that leaves BottomRight's last
    // row only partially visible (590 px canvas, 20 px rows, 29 px header
    // origin: row 29 spans 589..609, clipped to 1 px inside the 590 px
    // canvas) — `fill_axis`'s ceiling rule still carries one further row
    // past that (row 30, fully off-canvas), giving a cached range (1, 30)
    // whose row count (29) differs from the live_canvas sequence below.
    let stale_canvas = CanvasSize { w: 600.0, h: 590.0 };
    let stale_inputs = test_inputs(&m, stale_canvas, &theme);
    let stale_frame = Chrome::next(None, &m, &stale_inputs, FramePath::Fresh);
    core.render_grid(&m, &stale_frame, PaneRegionMask::ALL);
    let stale_range = core.pane_cache.pane(PaneRegion::BottomRight).range.get();
    assert_eq!(
        stale_range,
        Some(iron_canvas_core::RCRange {
            r1: 1,
            c1: 1,
            r2: 30,
            c2: 9
        }),
        "the 590px-canvas seed must land on the derived (1,30) range (29 \
         rows) — if this drifts, the geometry comment above is stale"
    );

    // The scroll/blit sequence itself is entirely at `canvas()` (600x400,
    // evenly divisible by the 20 px rows) — internally consistent, so it
    // blits cleanly on its own; the SAME `core` (and so the SAME pane cache,
    // still holding the 590-shaped range above) is reused across both.
    let live_canvas = canvas();
    let inputs = test_inputs(&m, live_canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);

    m.set_top_row(2);
    let inputs = test_inputs(&m, live_canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "uniform single-row scroll must qualify for blit",
    );
    let inputs = test_inputs(&m, live_canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    assert_eq!(
        PaneRegion::BottomRight.range(&frame1),
        Some(iron_canvas_core::RCRange {
            r1: 2,
            c1: 1,
            r2: 21,
            c2: 9
        }),
        "the 400px-canvas 1-row scroll must land on the derived (2,21) range \
         (19 rows) — the mismatch against the (1,30)/29-row seed above is the \
         IncompatibleRange trigger this test exercises"
    );

    core.reset_trace();
    let held = core.render_grid_blit(&m, &frame1, &plan);
    assert!(!held, "a healthy bridge must not abort the frame");

    let trace = core.trace();
    assert!(
        matches!(
            trace.panes[PaneRegion::BottomRight as usize],
            Some(PaneVerdict::Full)
        ),
        "an IncompatibleRange-demoted pane must repaint Full, got {:?}",
        trace.panes[PaneRegion::BottomRight as usize]
    );
    assert!(
        trace
            .blit_fallback
            .is_some_and(|fb| fb.pane == PaneRegion::BottomRight && !fb.cold_cache),
        "the fallback must be attributed to range incompatibility, not a cold cache"
    );
}

/// Fix B regression (blit atomicity): a `BridgeFailed` fetch on the revealed
/// strip must abort the WHOLE blit frame BEFORE any pixel is shifted, not
/// shift the kept band and only then discover the fetch failed (which strands
/// stale, now-misplaced pixels in the revealed strip with nothing to repaint
/// them). Drives the real gated sequence `RendererCore::render_grid_blit` runs
/// internally: every pane prepared and bridge-validated first, and pixels
/// shift only once every fetch is confirmed clean.
#[test]
fn blit_preflight_bridge_failure_aborts_frame_without_shifting() {
    let m = TestModel::synthetic_grid();
    m.set_data_until(30); // Real content, so a stray paint would be visible.
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);

    let range_before = core.pane_cache.pane(PaneRegion::BottomRight).range.get();
    assert!(
        range_before.is_some(),
        "Fresh must prime BottomRight's cached range"
    );

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-row scroll must qualify for blit",
    );
    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };

    // On the blit path only the revealed strip is fetched (the kept band is
    // preserved by the pixel blit), so failing all value fetches from here
    // fails exactly — and only — the strip fetch.
    m.set_value_bridge_fail(true);

    let baseline_ops = core.painter().ops().len();

    let held = core.render_grid_blit(&m, &frame1, &plan);

    assert!(
        held,
        "a BridgeFailed fetch on the revealed strip must abort the blit frame"
    );

    let new_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();
    assert_eq!(
        count_blits(&new_ops),
        0,
        "an aborted blit frame must shift zero pixels, got: {new_ops:#?}"
    );
    assert_eq!(
        count_rect_fills(&new_ops),
        0,
        "an aborted blit frame must not clear or paint the would-have-been-revealed strip"
    );
    assert!(
        new_ops.is_empty(),
        "an aborted blit frame must be a complete no-op for the grid layer, got: {new_ops:#?}"
    );

    // Cache untouched: the deferred `PaneBuffers::apply_shift` never ran, so
    // the pane's buffers weren't rotated and its cached range wasn't advanced.
    assert_eq!(
        core.pane_cache.pane(PaneRegion::BottomRight).range.get(),
        range_before,
        "an aborted blit frame must leave the pane's cached range exactly as it was"
    );
}

#[test]
fn overlap_row_height_change_disqualifies_blit() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    // Frame 0 sees row 5 at the default 20 px height.
    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);

    // Resize row 5 between frames AND scroll. Row 5 sits inside the
    // overlap band of a 1-row scroll, so `Chrome::classify`'s overlap probe
    // must fail and the fast-path must bail to a full repaint.
    m.set_row_height(5, 40.0);
    m.set_top_row(2);

    let inputs = test_inputs(&m, canvas, &theme);
    let delta = Chrome::classify(Some(&frame0), &m, &inputs, Some(&snap(&m)));
    assert!(
        matches!(
            delta,
            FrameDelta::Rebuild(RebuildReason::IncompatibleScrollOverlap)
        ),
        "row-height mutation inside the kept band must disqualify the blit",
    );
}

/// Regression for the smearing bug seen in the browser: data ends inside
/// the viewport (rows 1..=15 have data, 16+ empty), user scrolls by one
/// row. The strip is row 21 (newly revealed, empty); the kept band rows
/// 2..=20 had their pixels preserved by `Painter::blit`. The strip-fetch
/// path must not emit `FillText` ops for the kept band — doing so would
/// re-paint cells the blit already placed correctly, and (visually) drag
/// the last data row's text into rows below it.
///
/// Assertion: after the scroll-blit, no `FillText` op carries a `"R{n}"`
/// data-cell text. Strip cells are empty so they emit no text; kept-band
/// cells were preserved so they emit no text. The total post-scroll
/// FillText count for data-shaped strings must be zero.
#[test]
fn scroll_blit_does_not_smear_last_data_row_into_strip() {
    let m = TestModel::synthetic_grid();
    m.set_data_until(15);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);
    let baseline_ops = core.painter().ops().len();

    m.set_top_row(2);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        &m,
        &inputs,
        "single-row scroll must qualify for blit",
    );

    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    let post_scroll_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    let data_text_ops: Vec<&DrawOp> = post_scroll_ops
        .iter()
        .filter(|op| match op {
            DrawOp::FillText { text, .. } => text.starts_with('R'),
            _ => false,
        })
        .collect();

    assert!(
        data_text_ops.is_empty(),
        "scroll-blit must not re-paint kept-band cells; got {} FillText ops with data text: {:#?}",
        data_text_ops.len(),
        data_text_ops,
    );
}

/// Variant: 5-row scroll where data ends right at the last visible row
/// of the *initial* frame (canvas shows 20 rows, data_until = 20). After
/// the scroll the last data row is mid-viewport and strip rows 21..=25
/// reveal newly-visible empty cells. Mirrors the screenshot scenario:
/// last visible row had data pre-scroll, new strip below is empty.
#[test]
fn scroll_blit_does_not_smear_when_data_ends_at_initial_last_visible_row() {
    let m = TestModel::synthetic_grid();
    m.set_data_until(20);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();

    let inputs = test_inputs(&m, canvas, &theme);
    let frame0 = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(&m, &frame0, PaneRegionMask::ALL);
    let baseline_ops = core.painter().ops().len();

    m.set_top_row(6);
    let inputs = test_inputs(&m, canvas, &theme);
    let plan = qualify_scroll(&frame0, &m, &inputs, "5-row scroll must qualify for blit");

    let inputs = test_inputs(&m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), &m, &inputs, &plan) else {
        panic!("single-row scroll must blit in place");
    };
    core.render_grid_blit(&m, &frame1, &plan);

    let post_scroll_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(baseline_ops)
        .cloned()
        .collect();

    // Non-aligned axis: (400 − 28) / 20 = 18.6, so prev had two transition
    // rows. R20 was the overflow (top past canvas bottom) and becomes fully
    // visible in new — never blitted, must repaint. R19 was prev's partial
    // (12 px visible) and is partial again in new at a different fraction
    // (8 px overlap the strip clip), so its bottom band needs new pixels
    // too. Any *other* R-text is a kept-band smear.
    let smeared_text_ops: Vec<&DrawOp> = post_scroll_ops
        .iter()
        .filter(|op| match op {
            DrawOp::FillText { text, .. } => {
                text.starts_with('R') && text != "R19" && text != "R20"
            }
            _ => false,
        })
        .collect();

    assert!(
        smeared_text_ops.is_empty(),
        "5-row scroll-blit must not re-paint kept-band data cells; got {} ops: {:#?}",
        smeared_text_ops.len(),
        smeared_text_ops,
    );
}

// ============================================================================
// Stage 1 — BlitPaneWork construction
//
// Drive a Fresh frame to prime the pane cache, scroll one axis, qualify the
// blit, then build the per-pane `BlitPaneWork` exactly as `render_grid_blit`
// does (cache emits address-space work, the renderer-local helper widens it +
// attaches the pixel clip). Assert the address-space `strip_range` and the
// `pixel_clip` separately.
// ============================================================================

/// Build the `BlitPaneWork` for `pane` the same way `render_grid_blit` does:
/// the cache emits address-space work read off the *pre-shift* cached range,
/// then the renderer-local helper widens it against `frame1`'s slot geometry.
/// Returns both halves so tests can assert the base (pre-widen) strip — where
/// the overflow-row carry lives — and the widened strip + clip separately.
/// Returns `None` when the pane has no cache / an incompatible range (the
/// production fall-back-to-`render_pane` path).
fn build_pane_work(
    core: &RendererCore<RecorderPainter>,
    frame1: &Chrome,
    plan: &iron_canvas_core::chrome::BlitPlan,
    pane: PaneRegion,
) -> Option<(PaneBlitAddressWork, BlitPaneWork)> {
    let new_range = pane.range(frame1)?;
    let PaneShiftPrep::Shifted {
        prev_range,
        new_range,
    } = core
        .pane_cache
        .pane(pane)
        .classify_shift(new_range, plan.axis)
    else {
        return None;
    };
    let address_work = core
        .pane_cache
        .plan_blit_pane(prev_range, new_range, plan.axis)?;
    let work = widen_blit_strip_to_pixel_clip(frame1, plan, pane, address_work);
    Some((address_work, work))
}

/// Drive Fresh frame 0 (priming the pane cache), apply `scroll`, qualify the
/// blit, and hand back the live core + frame1 + plan for work construction.
fn primed_blit(
    m: &TestModel,
    scroll: impl FnOnce(&TestModel),
) -> (
    RendererCore<RecorderPainter>,
    Chrome,
    iron_canvas_core::chrome::BlitPlan,
) {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let canvas = canvas();
    let inputs = test_inputs(m, canvas, &theme);
    let frame0 = Chrome::next(None, m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(m, &frame0, PaneRegionMask::ALL);

    scroll(m);
    let inputs = test_inputs(m, canvas, &theme);
    let plan = qualify_scroll(
        &frame0,
        m,
        &inputs,
        "single-axis scroll must qualify for blit",
    );
    let inputs = test_inputs(m, canvas, &theme);
    let BlitOutcome::Blitted(frame1) = Chrome::next_blit(Some(frame0), m, &inputs, &plan) else {
        panic!("single-axis scroll must blit in place");
    };
    (core, frame1, plan)
}

#[test]
fn row_scroll_bottom_right_work_has_expected_strip() {
    let m = TestModel::synthetic_grid();
    let (core, frame1, plan) = primed_blit(&m, |m| m.set_top_row(2));

    let (base, work) = build_pane_work(&core, &frame1, &plan, PaneRegion::BottomRight)
        .expect("BottomRight has a cached range to shift");

    // Base (pre-widen) strip: spans the new pane's full column extent and,
    // along the scroll axis, begins at the prev overflow row.
    assert_eq!(base.strip_range.c1, base.new_range.c1);
    assert_eq!(base.strip_range.c2, base.new_range.c2);
    assert_eq!(
        base.strip_range.r1, base.prev_range.r2,
        "base down-scroll strip must start at prev.r2 (the off-canvas overflow row)",
    );
    assert_eq!(base.strip_range.r2, base.new_range.r2);

    // Widening only grows the strip toward the canvas edge — never past the
    // overflow row — and the main scroll pane carries the pixel clip.
    assert!(work.strip_range.r1 <= base.strip_range.r1);
    assert_eq!(
        work.pixel_clip,
        Some(plan.repaint_strip),
        "BottomRight work must carry the repaint-strip pixel clip",
    );
}

#[test]
fn col_scroll_bottom_right_work_has_expected_strip() {
    let m = TestModel::synthetic_grid();
    let (core, frame1, plan) = primed_blit(&m, |m| m.set_left_column(2));

    let (base, work) = build_pane_work(&core, &frame1, &plan, PaneRegion::BottomRight)
        .expect("BottomRight has a cached range to shift");

    // Base strip spans the row extent unchanged and along the scroll axis
    // begins at the prev overflow column.
    assert_eq!(base.strip_range.r1, base.new_range.r1);
    assert_eq!(base.strip_range.r2, base.new_range.r2);
    assert_eq!(
        base.strip_range.c1, base.prev_range.c2,
        "base right-scroll strip must start at prev.c2 (the off-canvas overflow column)",
    );
    assert_eq!(base.strip_range.c2, base.new_range.c2);

    assert!(work.strip_range.c1 <= base.strip_range.c1);
    assert_eq!(
        work.pixel_clip,
        Some(plan.repaint_strip),
        "BottomRight work must carry the repaint-strip pixel clip",
    );
}

#[test]
fn row_scroll_with_frozen_cols_includes_bottom_left_work() {
    let m = TestModel::synthetic_grid().with_frozen_cols(2);
    let (core, frame1, plan) = primed_blit(&m, |m| m.set_top_row(2));

    // A row scroll with frozen columns shifts BottomLeft (the frozen-col
    // band) alongside BottomRight.
    assert!(
        plan.shift_panes().contains_region(PaneRegion::BottomLeft),
        "row scroll with frozen cols must shift BottomLeft",
    );

    let (base, work) = build_pane_work(&core, &frame1, &plan, PaneRegion::BottomLeft)
        .expect("BottomLeft has a cached range to shift");

    assert_eq!(
        base.strip_range.r1, base.prev_range.r2,
        "frozen-band base strip must also carry the overflow row",
    );
    // Frozen-band sibling paints its narrowed range with no extra clip.
    assert_eq!(
        work.pixel_clip, None,
        "BottomLeft frozen-band work must not carry a pixel clip",
    );
}

#[test]
fn col_scroll_with_frozen_rows_includes_top_right_work() {
    let m = TestModel::synthetic_grid().with_frozen_rows(2);
    let (core, frame1, plan) = primed_blit(&m, |m| m.set_left_column(2));

    assert!(
        plan.shift_panes().contains_region(PaneRegion::TopRight),
        "col scroll with frozen rows must shift TopRight",
    );

    let (base, work) = build_pane_work(&core, &frame1, &plan, PaneRegion::TopRight)
        .expect("TopRight has a cached range to shift");

    assert_eq!(
        base.strip_range.c1, base.prev_range.c2,
        "frozen-band base strip must also carry the overflow column",
    );
    assert_eq!(
        work.pixel_clip, None,
        "TopRight frozen-band work must not carry a pixel clip",
    );
}

#[test]
fn down_scroll_strip_includes_overflow_row() {
    let m = TestModel::synthetic_grid();
    let (core, frame1, plan) = primed_blit(&m, |m| m.set_top_row(2));

    let (base, _work) = build_pane_work(&core, &frame1, &plan, PaneRegion::BottomRight)
        .expect("BottomRight has a cached range to shift");

    // The defining overflow-row invariant: the revealed band starts at
    // prev.r2 (the off-canvas overflow row the blit never shifted), NOT
    // prev.r2 + 1.
    assert_eq!(base.strip_range.r1, base.prev_range.r2);
    assert_ne!(base.strip_range.r1, base.prev_range.r2 + 1);
}

// ============================================================================
// Stage 2 — typed shift prep
//
// Drive `PaneBuffers::classify_shift` (pure decision) and
// `PaneBuffers::apply_shift` (execution-only rotation — Stage 4 split the old
// combined `prepare_shift` in two, see that method's doc) directly against a
// hand-seeded `PaneCache` so the typed result and the in-place buffer
// rotation can be asserted in isolation from frame/pixel geometry. The
// rotation tests capture the expected post-shift buffer contents explicitly
// (computed by hand from `apply_blit_shift`'s contract) — bit-identical to
// what the old `try_shift(..) == true` path produced.
// ============================================================================

use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::renderer::cache::PaneCache;

fn rng(r1: i32, r2: i32, c1: i32, c2: i32) -> iron_canvas_core::RCRange {
    iron_canvas_core::RCRange { r1, r2, c1, c2 }
}

fn val(s: &str) -> iron_canvas_core::Fetched<String> {
    iron_canvas_core::Fetched::Value(s.to_string())
}

/// `apply_shift` rotates all four pane buffers in lockstep, so every buffer
/// must enter at the prev range's slot count (`apply_blit_shift` debug-asserts
/// it). The rotation tests only inspect `values`; seed the other three to the
/// same length with placeholders so the shift is well-formed.
fn seed_sibling_buffers(pane: &iron_canvas_core::renderer::cache::PaneBuffers, len: usize) {
    pane.styles
        .set(vec![iron_canvas_core::Fetched::Absent; len]);
    pane.cell_types
        .set(vec![iron_canvas_core::Fetched::Absent; len]);
    pane.decorations
        .set(vec![iron_canvas_core::Fetched::Absent; len]);
}

#[test]
fn classify_shift_reports_missing_cache() {
    let cache = PaneCache::default();
    // No `range` seeded -> cache is empty.
    let prep = cache
        .pane(PaneRegion::BottomRight)
        .classify_shift(rng(2, 3, 1, 2), Axis::Row);
    assert_eq!(prep, PaneShiftPrep::MissingCache);
}

#[test]
fn classify_shift_reports_incompatible_range() {
    let cache = PaneCache::default();
    let pane = cache.pane(PaneRegion::BottomRight);
    let prev = rng(1, 2, 1, 2);
    pane.range.set(Some(prev));

    // Row scroll but the orthogonal (column) extent changed -> incompatible.
    let new = rng(2, 3, 1, 5);
    let prep = pane.classify_shift(new, Axis::Row);
    assert_eq!(
        prep,
        PaneShiftPrep::IncompatibleRange {
            prev_range: prev,
            new_range: new,
        },
    );
    // Pure: classification alone must never clear (or otherwise touch) the
    // cached range — that decision belongs to whichever caller consumes
    // `IncompatibleRange` (a full-pane fallback fetch, which only ever
    // overwrites `range` at commit time).
    assert_eq!(pane.range.get(), Some(prev));
}

#[test]
fn apply_shift_rotates_row_buffers() {
    let cache = PaneCache::default();
    let pane = cache.pane(PaneRegion::BottomRight);
    let prev = rng(1, 2, 1, 2);
    pane.range.set(Some(prev));
    // 2×2 row-major: (1,1)(1,2)(2,1)(2,2).
    pane.values
        .set(vec![val("a"), val("b"), val("c"), val("d")]);
    seed_sibling_buffers(pane, 4);

    // Scroll down by one row: delta = +1, shift = 1 row × 2 cols = 2.
    // rotate_left(2) -> [c,d,a,b]; fill the trailing strip (last 2) -> Absent.
    let new = rng(2, 3, 1, 2);
    // Classification confirms this is a legal `Shifted` rotation before it
    // runs — mirrors production's prepare (classify_shift) -> execute
    // (apply_shift, only once the revealed strip's fetch is already clean).
    assert_eq!(
        pane.classify_shift(new, Axis::Row),
        PaneShiftPrep::Shifted {
            prev_range: prev,
            new_range: new,
        },
    );

    pane.apply_shift(prev, new, Axis::Row);

    let expected = vec![
        val("c"),
        val("d"),
        iron_canvas_core::Fetched::Absent,
        iron_canvas_core::Fetched::Absent,
    ];
    let got = pane.values.take();
    assert_eq!(
        got, expected,
        "row rotation must be bit-identical to try_shift"
    );

    // `apply_shift` rotates buffers only; committing `range` to `new` is
    // execution's separate, later step (via `RendererCore::commit_pane_cache`),
    // never `apply_shift`'s own job.
    assert_eq!(pane.range.get(), Some(prev));
}

#[test]
fn apply_shift_rotates_column_buffers() {
    let cache = PaneCache::default();
    let pane = cache.pane(PaneRegion::BottomRight);
    let prev = rng(1, 2, 1, 2);
    pane.range.set(Some(prev));
    // 2×2 row-major: rows [a,b] / [c,d].
    pane.values
        .set(vec![val("a"), val("b"), val("c"), val("d")]);
    seed_sibling_buffers(pane, 4);

    // Scroll right by one column: delta = +1, each row rotate_left(1), fill
    // the trailing column with Absent -> rows [b,Absent] / [d,Absent].
    let new = rng(1, 2, 2, 3);
    assert_eq!(
        pane.classify_shift(new, Axis::Column),
        PaneShiftPrep::Shifted {
            prev_range: prev,
            new_range: new,
        },
    );

    pane.apply_shift(prev, new, Axis::Column);

    let expected = vec![
        val("b"),
        iron_canvas_core::Fetched::Absent,
        val("d"),
        iron_canvas_core::Fetched::Absent,
    ];
    let got = pane.values.take();
    assert_eq!(
        got, expected,
        "column rotation must be bit-identical to try_shift"
    );
    assert_eq!(pane.range.get(), Some(prev));
}

// ============================================================================
// Task 2 — `render_pane_damage`'s range-mismatch demotion
//
// The orchestrator's public setters cannot manufacture a genuine
// `pane_buf.range` mismatch: the `Damage` regime only dispatches while
// the pending content holds `ContentWork::Rows{..}`, and that state can
// only become fresh again after a fully successful prior paint — which
// itself re-populates
// every touched pane's cached range in lockstep with the frame it just
// built. Driven directly here instead: a virgin `RendererCore` has never
// set `pane_buf.range` at all, so the very first `render_pane_damage` call
// sees a guaranteed mismatch against the pane's real (`Some`) range and
// takes the demotion branch — proving it forwards `render_pane`'s bool
// rather than swallowing it.
// ============================================================================

#[test]
fn damage_range_mismatch_demotes_to_render_pane_and_forwards_its_hold() {
    let m = TestModel::synthetic_grid();
    m.set_bulk_bridge_fail(true);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    // `reuses_slots()` gates `render_pane`'s hold branch — without this the
    // demoted `render_pane` would paint blanks instead of holding.
    frame.kind = FrameKindTag::SlotsReused;
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    let held = core.render_pane_damage(
        &m,
        &frame,
        PaneRegion::BottomRight,
        &[RowSpan { r1: 1, r2: 1 }],
    );
    assert!(
        held,
        "a range mismatch must demote to render_pane and forward its Held bool"
    );
    assert!(
        core.painter().ops().is_empty(),
        "a held demotion must paint nothing"
    );

    m.set_bulk_bridge_fail(false);
    let held = core.render_pane_damage(
        &m,
        &frame,
        PaneRegion::BottomRight,
        &[RowSpan { r1: 1, r2: 1 }],
    );
    assert!(
        !held,
        "once the bridge recovers, the demoted render_pane must paint and report not-held"
    );
    assert!(!core.painter().ops().is_empty());
}

// ============================================================================
// Review finding: `render_pane_damage`'s span loop must stop at the FIRST
// held span within one pane and never attempt a later sibling span. The
// orchestrator-level "multi span" test in `held_frame.rs` only ever routes
// each span to a DIFFERENT pane (its frozen-row seam lines up with its fail
// threshold), so within any one pane's call the sibling span always has an
// empty row intersection and is skipped via the `r1 > r2` guard before ever
// reaching `render_pane_strip` — it re-proves the cross-pane OR-fold, not
// this intra-pane early return. Pinned directly here: one pane, two REAL
// spans, ordered failing-then-healthy.
// ============================================================================

#[test]
fn render_pane_damage_stops_at_first_held_span_in_one_pane() {
    let m = TestModel::synthetic_grid().with_data_until(10);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas(), &theme);
    let frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Prime BottomRight's cached range via an ordinary Fresh paint so the
    // damage call below takes the span loop, not the range-mismatch
    // demotion the sibling test above already covers.
    core.render_grid(&m, &frame, PaneRegionMask::ALL);

    m.set_bulk_bridge_fail_from(Some(5));
    m.reset_bulk_fetch_calls();
    let ops_before = core.painter().ops().len();

    // Failing span (r1=5) ordered FIRST, healthy span (r1=3) SECOND: the
    // loop must stop at the first hold and never reach the second span.
    let held = core.render_pane_damage(
        &m,
        &frame,
        PaneRegion::BottomRight,
        &[RowSpan { r1: 5, r2: 5 }, RowSpan { r1: 3, r2: 3 }],
    );

    assert!(held, "a held first span must mark the whole pane call held");
    assert_eq!(
        m.bulk_fetch_calls(),
        4,
        "exactly one strip fetch (the four bulk accessors) — the healthy \
         second span must never be fetched once the loop holds on the first"
    );
    let new_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(ops_before)
        .cloned()
        .collect();
    assert!(
        !new_ops
            .iter()
            .any(|op| matches!(op, DrawOp::FillText { text, .. } if text == "R3")),
        "the second (healthy) span's row must never paint once the loop \
         holds on the first — got {new_ops:#?}"
    );
}

// ==============================================================================
// Stage 4 pin (Task 1, bullet 7): the mirror image of
// `render_pane_damage_stops_at_first_held_span_in_one_pane` above — there,
// the FIRST span fails and the loop stops before ever reaching the second.
// Here the FIRST span's fetch would succeed on its own, and the SECOND
// (same pane) fails. Stage 4 requires every intersecting strip in one pane
// to be prepared atomically before any of them paints, so a failure
// anywhere in the pane must leave the whole pane exactly as it was —
// including the span that, looked at alone, was fine.
// ==============================================================================

/// RED against d8aed9c: `render_pane_damage`'s loop calls `render_pane_strip`
/// once PER span, and each call splices its fetch into the cached pane
/// buffers and paints immediately on success — there is no pane-wide
/// preparation step before any span commits. So the first span's ops and
/// cache splice land before the second span's failure is ever discovered.
#[test]
fn render_pane_damage_prepares_whole_pane_atomically_before_painting_any_span() {
    let m = TestModel::synthetic_grid().with_data_until(10);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas(), &theme);
    let frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Prime BottomRight's cached range via an ordinary Fresh paint so the
    // damage call below takes the span loop, not the range-mismatch
    // demotion `damage_range_mismatch_demotes_to_render_pane_and_forwards_its_hold`
    // already covers.
    core.render_grid(&m, &frame, PaneRegionMask::ALL);

    m.set_cell(3, 1, "span-a");
    m.set_cell(7, 1, "span-b");
    m.set_bulk_bridge_fail_from(Some(5)); // span-a's row (3) fetches OK; span-b's (7) fails.

    let values_before = core.pane_cache.pane(PaneRegion::BottomRight).values.take();
    core.pane_cache
        .pane(PaneRegion::BottomRight)
        .values
        .set(values_before.clone());
    let ops_before = core.painter().ops().len();

    let held = core.render_pane_damage(
        &m,
        &frame,
        PaneRegion::BottomRight,
        &[RowSpan { r1: 3, r2: 3 }, RowSpan { r1: 7, r2: 7 }],
    );

    assert!(
        held,
        "the second span's failure must mark the whole call held"
    );
    let new_ops: Vec<DrawOp> = core
        .painter()
        .ops()
        .iter()
        .skip(ops_before)
        .cloned()
        .collect();
    assert!(
        new_ops.is_empty(),
        "the first (healthy) span must not paint once a later span in the \
         same pane fails — atomic per-pane preparation; got {new_ops:#?}"
    );
    let values_after = core.pane_cache.pane(PaneRegion::BottomRight).values.take();
    core.pane_cache
        .pane(PaneRegion::BottomRight)
        .values
        .set(values_after.clone());
    assert_eq!(
        values_after, values_before,
        "the first span's fetch must not be spliced into the cached pane \
         buffers once a later span in the same pane fails"
    );
    let recycled = core.strip_scratch_capacities();
    assert!(
        recycled.len() >= 2,
        "both the successful first strip and failing second strip must return to the pool: {recycled:?}"
    );
    assert!(
        recycled
            .iter()
            .all(|caps| caps.0 > 0 && caps.1 > 0 && caps.2 > 0 && caps.3 > 0),
        "every aborted strip bundle must retain all four channel capacities: {recycled:?}"
    );
}
