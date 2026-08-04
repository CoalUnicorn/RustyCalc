//! Stage 4 pins (Task 1, bullet 9): buffer/range/fingerprint state must not
//! advance on a failed prepare — captured entirely against Stage 3's
//! (`d8aed9c`) current mechanism, before `PreparedPane` / `PreparedCacheCommit`
//! exist. Stage 4 replaces *how* a pane is prepared (an owned typed value
//! rather than mutating `pane_buf` in place and hoping a failure is caught
//! before any of the four `Cell`s are `.set()` back with new content) but
//! must keep these exact equalities true across the refactor.
//!
//! Two prepare sites are pinned here: `render_pane`'s full-pane fetch (the
//! SlotsReuse/Fresh path) and `render_pane_damage`'s strip fetch (via
//! `prepare_damage_pane`). The third prepare site — the blit preflight's
//! per-pane strip fetch, inline in `prepare_blit`'s own loop — already has
//! equivalent range-level (not full four-buffer) coverage in
//! `scroll_blit.rs`'s
//! `blit_preflight_bridge_failure_aborts_frame_without_shifting` and
//! `cold_cache_bridge_failure_holds_the_whole_blit_frame`; not duplicated
//! here.
//!
//! The fingerprint tree itself (`PaneFingerprintState`) is `pub(crate)` —
//! invisible from an integration test — so "the fingerprint digest is
//! unchanged" is proven the same indirect way every sibling file in this
//! crate proves it: an unchanged-content repaint immediately afterward must
//! Skip. A corrupted or reset tree would either mismatch (repaint instead of
//! skipping) or panic; a clean Skip is the observable proof.
//!
//! GREEN today: Stage 3 already gets single-call atomicity right at both
//! prepare sites below (see `render_pane`'s and `prepare_damage_pane`'s own
//! doc comments) — but ONLY on a `SlotsReused`/`Blitted`-kind frame. The
//! third and fourth tests in this file isolate a further, more fundamental
//! finding Stage 4's own TDD process surfaced while writing the
//! Fresh-atomicity pins in `held_frame.rs`: `render_pane`'s bridge-hold
//! branch is gated behind `frame.kind.reuses_slots()`, which is always
//! `false` for a `FramePath::Fresh` candidate — so on a Fresh-kind frame,
//! `render_pane` *itself* never detects a bulk bridge failure at all; it
//! paints through with the missing cells silently rendered blank and
//! reports `held = false` (success). That gap is real, but it is not
//! `render_pane`'s to close: Task 5 gives `Fresh` its OWN atomicity gate
//! instead, as a genuinely separate prepare/execute pair —
//! `RendererCore::prepare_fresh_panes` (pure: bulk fetch + bridge check,
//! zero painter interaction) and `RendererCore::execute_fresh_grid`
//! (infallible: paints a bundle `prepare_fresh_panes` already confirmed
//! healthy) — reached through `LayerBase::paint_grid_fresh`, never through
//! `render_pane`. The two tests below exercise that actual mechanism
//! directly, at the same low level `render_pane`'s own sibling tests above
//! use, rather than asserting a hold on `render_pane` that (correctly)
//! never comes: Fresh's atomicity lives one level up, in whether
//! `prepare_fresh_panes` is even called with a bulk-fetch that fails.

mod common;

use iron_canvas_core::RowSpan;
use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath, PaneRegion, PaneRegionMask};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::renderer::cache::PaneBuffers;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CellDecoration, CellKind, CellStyle, Fetched, RCRange};
use iron_canvas_recorder::RecorderPainter;

use common::{TestModel, canvas_default, test_inputs};

fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.kind = FrameKindTag::SlotsReused;
}

type PaneSnapshot = (
    Vec<Fetched<CellStyle>>,
    Vec<Fetched<String>>,
    Vec<Fetched<CellKind>>,
    Vec<Fetched<CellDecoration>>,
    Option<RCRange>,
);

/// Non-destructive peek at one pane's cached range plus all four buffers:
/// `.take()` each `Cell` then immediately `.set()` the clone back, so
/// capturing the snapshot never itself mutates the state under test — the
/// same take/park rhythm `scroll_blit.rs`'s Stage 2 section uses for
/// `PaneBuffers::values` alone, extended here to all four buffers plus
/// `range` so the whole prepared unit can be compared in one `assert_eq!`.
fn snapshot(pane: &PaneBuffers) -> PaneSnapshot {
    let styles = pane.styles.take();
    let values = pane.values.take();
    let cell_types = pane.cell_types.take();
    let decorations = pane.decorations.take();
    let range = pane.range.get();
    pane.styles.set(styles.clone());
    pane.values.set(values.clone());
    pane.cell_types.set(cell_types.clone());
    pane.decorations.set(decorations.clone());
    (styles, values, cell_types, decorations, range)
}

/// Prepare site 1: `render_pane`'s full-pane bulk fetch (SlotsReuse/Fresh).
#[test]
fn slots_reuse_fetch_failure_leaves_range_and_buffers_untouched_and_survives_as_a_skip() {
    let m = TestModel::synthetic_grid();
    m.set_cell(1, 1, "primed");
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let mut frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let before = snapshot(core.pane_cache.pane(PaneRegion::BottomRight));

    m.set_bulk_bridge_fail(true);
    let ops_before = core.painter().ops().len();
    let held = core.render_pane(&m, PaneRegion::BottomRight, &frame);

    assert!(held, "a full bulk-fetch failure must hold this pane");
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "a held prepare must emit no ops"
    );
    let after = snapshot(core.pane_cache.pane(PaneRegion::BottomRight));
    assert_eq!(
        before, after,
        "a failed prepare must leave the pane's cached range and all four \
         buffers byte-identical to what they were before the attempt"
    );

    // Fingerprint-digest proxy: the tree is `pub(crate)` and unreachable
    // from here, so recovery with byte-identical content must Skip — a
    // corrupted or reset tree would mismatch and repaint instead.
    m.set_bulk_bridge_fail(false);
    let ops_before = core.painter().ops().len();
    let held = core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(!held, "recovery must not hold");
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "recovery with unchanged content must Skip — the painted tree must \
         have survived the failed prepare untouched"
    );
}

/// Bullet 1's "unchanged pane-cache committed ranges" clause, untestable at
/// the `Orchestrator` level (it exposes no `PaneCache`/`PaneBuffers`
/// accessor), pinned here at the `RendererCore` level instead, sibling to
/// the two prepare-site tests above. Primes `BottomRight` via one ordinary
/// healthy `render_grid_fresh` call, then fails a SECOND `render_grid_fresh`
/// call on the SAME pane — mirroring exactly what a real Fresh dispatch is,
/// first paint or a later rebuild alike.
///
/// Same shape as `slots_reuse_fetch_failure_leaves_range_and_buffers_untouched_and_survives_as_a_skip`
/// above: a held prepare touches `PaneBuffers`' four content fields and
/// `range` not at all, so every one of them is trivially byte-identical
/// before/after. Before Task 5 this test targeted `render_pane` directly
/// and found the OPPOSITE — real corruption, via `Fetched::take_value`'s
/// side effects running unconditionally on a Fresh-kind frame that never
/// held (see git history for that mechanism if it resurfaces). It cannot
/// happen through `render_grid_fresh` (`prepare_fresh_panes` +
/// `execute_fresh_grid` under one call), which never runs `render_pane`'s
/// per-cell consuming walk on a failed prepare at all — Fresh's atomicity
/// now lives one level up, in whether execution runs at all.
#[test]
fn fresh_kind_frame_bridge_failure_leaves_the_cached_pane_buffers_untouched() {
    let m = TestModel::synthetic_grid();
    m.set_cell(1, 1, "primed");
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    let mask = PaneRegionMask::EMPTY.with(PaneRegion::BottomRight);

    // Prime: an ordinary, healthy Fresh prepare+execute cycle establishes
    // real cached content — this is the "committed" state bullet 1 says
    // must survive.
    let held = core.render_grid_fresh(&m, &frame, mask);
    assert!(!held, "priming paint must not hold");
    let (before_styles, before_values, before_cell_types, before_decorations, before_range) =
        snapshot(core.pane_cache.pane(PaneRegion::BottomRight));

    // Second Fresh prepare on the SAME pane, deliberately left at Fresh
    // kind (never promoted to SlotsReused) — this is exactly what a real
    // Fresh dispatch is, first paint or a later rebuild alike.
    m.set_bulk_bridge_fail(true);
    let ops_before = core.painter().ops().len();
    let held = core.render_grid_fresh(&m, &frame, mask);
    let (after_styles, after_values, after_cell_types, after_decorations, after_range) =
        snapshot(core.pane_cache.pane(PaneRegion::BottomRight));

    assert!(
        held,
        "Stage 4: a Fresh-kind frame's atomic pane preparation must hold \
         when a targeted pane's bulk fetch fails"
    );
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "a held prepare must emit no ops"
    );
    // Field-by-field (not one opaque tuple `assert_eq!`) so a divergence in
    // just one of the five channels is legible instead of a multi-thousand-
    // token wall of `Absent`/`BridgeFailed`.
    assert_eq!(
        before_range, after_range,
        "a held prepare must leave the cached range untouched"
    );
    assert_eq!(
        before_styles, after_styles,
        "a held prepare must leave the cached styles buffer untouched"
    );
    assert_eq!(
        before_values, after_values,
        "a held prepare must leave the cached values buffer untouched"
    );
    assert_eq!(
        before_cell_types, after_cell_types,
        "a held prepare must leave the cached cell-types buffer untouched"
    );
    assert_eq!(
        before_decorations, after_decorations,
        "a held prepare must leave the cached decorations buffer untouched"
    );
}

#[test]
fn late_fresh_pane_failure_recycles_every_prepared_capacity() {
    let m = TestModel::synthetic_grid()
        .with_data_until(12)
        .with_frozen_rows(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    m.set_bulk_bridge_fail_from(Some(3));
    assert!(
        core.render_grid_fresh(&m, &frame, PaneRegionMask::ALL),
        "the scroll pane must fail after the frozen pane prepared successfully"
    );

    for pane in [PaneRegion::TopRight, PaneRegion::BottomRight] {
        let pane_buf = core.pane_cache.pane(pane);
        let (cells, _) = pane_buf.preparation_scratch_capacities();
        assert!(
            cells.0 > 0 && cells.1 > 0 && cells.2 > 0 && cells.3 > 0,
            "{pane:?} must return every fetched channel's capacity on abort: {cells:?}"
        );
        let fingerprint_rows = pane_buf.fingerprint_row_scratch_capacity();
        assert!(
            fingerprint_rows > 0,
            "{pane:?} must return its candidate fingerprint row capacity on abort: \
             {fingerprint_rows}"
        );
    }
}

/// Prepare site 2: `render_pane_damage`'s strip fetch (via
/// `prepare_damage_pane`).
#[test]
fn damage_strip_failure_leaves_range_and_buffers_untouched_and_survives_as_a_skip() {
    let m = TestModel::synthetic_grid();
    m.set_cell(1, 1, "primed");
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

    let before = snapshot(core.pane_cache.pane(PaneRegion::BottomRight));

    m.set_value_bridge_fail(true);
    let ops_before = core.painter().ops().len();
    let held = core.render_pane_damage(&m, &frame, PaneRegion::BottomRight, &[span]);

    assert!(held, "a strip fetch failure must hold this pane");
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "a held strip prepare must emit no ops"
    );
    let after = snapshot(core.pane_cache.pane(PaneRegion::BottomRight));
    assert_eq!(
        before, after,
        "a failed strip prepare must leave the pane's cached range and all \
         four buffers byte-identical to what they were before the attempt"
    );

    m.set_value_bridge_fail(false);
    let ops_before = core.painter().ops().len();
    let held = core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(!held, "recovery must not hold");
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "recovery with unchanged content must Skip — the painted tree must \
         have survived the failed strip prepare untouched"
    );
}

/// Isolates, at the lowest possible level, the finding described in this
/// file's module doc — `render_pane`'s own bridge-hold branch requires
/// `frame.kind.reuses_slots()`, which a `FramePath::Fresh` frame never
/// satisfies, so `render_pane` itself never holds on a Fresh-kind frame.
/// Fresh's atomicity is Task 5's job, given to it as a separate gate:
/// `RendererCore::render_grid_fresh`, exercised here directly. `frame` is
/// left at its constructed `Fresh` kind (never promoted to `SlotsReused`),
/// matching both the very first paint of an `Orchestrator`'s lifetime and
/// every later Fresh rebuild.
#[test]
fn fresh_kind_frame_detects_and_holds_on_a_bulk_bridge_failure() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&m, canvas_default(), &theme);
    let frame = Chrome::next(None, &m, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    m.set_bulk_bridge_fail(true);
    let mask = PaneRegionMask::EMPTY.with(PaneRegion::BottomRight);
    let held = core.render_grid_fresh(&m, &frame, mask);

    assert!(
        held,
        "a Fresh-kind frame's atomic pane preparation must hold when a \
         targeted pane's bulk fetch fails, exactly like a SlotsReuse-kind \
         frame's render_pane does"
    );
    assert!(
        core.painter().ops().is_empty(),
        "a held prepare must touch the painter not at all — not even a \
         group bracket — see RendererCore::prepare_fresh_panes's own doc \
         for why that atomicity has to start this early"
    );
}
