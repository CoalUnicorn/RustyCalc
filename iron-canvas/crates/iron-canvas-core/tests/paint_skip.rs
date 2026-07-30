//! Stage 1 fingerprint paint-skip — `render_pane` must emit zero `DrawOp`s
//! when the bulk-fetched buffers content-match the prior frame (under
//! `frame.kind == FrameKindTag::SlotsReused`), and must repaint exactly
//! the pane whose fingerprint changed.
//!
//! These tests target `render_pane` directly rather than `render_grid` so
//! the assertion surface stays the 4-pass per-pane walk. Header strips,
//! corner box, and frozen separators run above in `render_grid` and are
//! not fingerprint-gated.
//!
//! Painted-fingerprint ownership lives on `PaneCache`, which sits on
//! `RendererCore` — not `Chrome`. These tests must retain ONE `RendererCore`
//! across both frames of a scenario: a helper that builds a new
//! `RendererCore` per call would silently reset `PaneCache` and every
//! comparison would see an empty painted tree.

mod common;

use iron_canvas_core::RowSpan;
use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath, PaneRegion};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{
    Border, BorderItem, BorderStyle, CellDecoration, CellStyle, DataBarSpec, RCRange,
};
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, canvas_large};

/// Mirrors the orchestrator's `SlotsReuse` branch: just flip the kind tag
/// so the next `render_pane` call on the SAME `RendererCore` hits the
/// skip-comparison branch. Nothing on `Chrome` needs rotating anymore —
/// `PaneCache`'s per-pane `PaneFingerprintState` already persisted the
/// prior frame's committed tree across the call.
fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.kind = FrameKindTag::SlotsReused;
}

#[test]
fn render_pane_skips_on_idempotent_repaint() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // First paint runs through the full 4-pass walk; the kind is Fresh, so
    // the skip branch is gated off, but the painted tree is still seeded
    // on `core.pane_cache` for the next frame's compare.
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(
        !core.painter().ops().is_empty(),
        "first paint of a non-empty pane must emit ops"
    );

    promote_to_slots_reuse(&mut frame);

    // Model unchanged -> identical bulk-fetch buffers -> identical
    // fingerprint tree -> the entire 4-pass walk is skipped. No new ops
    // land in the (still-accumulating) recorder log.
    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "idempotent repaint under SlotsReused must skip render_pane entirely",
    );
}

#[test]
fn render_pane_skip_is_scoped_to_changed_pane() {
    // `frozen_cols = 2` splits the data-bearing region: BottomLeft owns
    // cols 1..=2, BottomRight owns cols 3..=. A mutation in one pane
    // must leave the other pane's fingerprint untouched.
    let m = TestModel::synthetic_grid().with_frozen_cols(2);
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomLeft, &frame);
    core.render_pane(&m, PaneRegion::BottomRight, &frame);

    promote_to_slots_reuse(&mut frame);

    // Col 5 lives past the frozen seam -> BottomRight only.
    m.set_cell(1, 5, "changed");

    let ops_before_bl = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomLeft, &frame);
    let ops_after_bl = core.painter().ops().len();

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops_after_br = core.painter().ops().len();

    assert_eq!(
        ops_after_bl, ops_before_bl,
        "unaffected pane must skip — per-pane fingerprint is the load-bearing claim",
    );
    assert!(ops_after_br > ops_after_bl, "mutated pane must repaint");
}

#[test]
fn slots_reuse_holds_prior_pane_through_two_consecutive_bridge_failures() {
    let m = TestModel::synthetic_grid();
    m.set_cell(1, 1, "still here");
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let painter = std::rc::Rc::new(RecorderPainter::new());
    let core = RendererCore::for_layer(std::rc::Rc::clone(&painter));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(
        !painter.ops().is_empty(),
        "first paint must populate prior pane pixels, buffers, and painted tree"
    );
    promote_to_slots_reuse(&mut frame);

    m.set_value_bridge_fail(true);
    let ops_after_first_paint = painter.ops().len();

    // First failure: the 4-way preflight (styles/values/cell_types/
    // decorations) must reject this frame's fetch and hold the prior
    // pixels — no clear, no repaint, and no touch to the painted-fingerprint
    // tree.
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        painter.ops().len(),
        ops_after_first_paint,
        "first BridgeFailed during SlotsReuse must hold prior pixels"
    );

    // Second, consecutive failure: must ALSO hold — proving the guard
    // (and the untouched pane buffers + painted tree behind it) survive
    // more than one bad frame in a row, not just a single transient one.
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        painter.ops().len(),
        ops_after_first_paint,
        "second consecutive BridgeFailed during SlotsReuse must still hold prior pixels"
    );

    // Recovery: the bridge returns with content IDENTICAL to what was
    // painted before the two failures. If either failure had corrupted the
    // pane buffers or the painted tree, this repaint would either draw
    // wrong pixels or fail to skip (a stale/absent tree would mismatch and
    // force a full repaint, emitting new ops). A clean skip here is the
    // proof that both the pane buffers and the painted tree survived two
    // consecutive bridge failures untouched.
    m.set_value_bridge_fail(false);
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        painter.ops().len(),
        ops_after_first_paint,
        "recovery repaint with unchanged content must skip cleanly — pane buffers and the \
         painted tree must have survived two consecutive bridge failures"
    );
}

// ==============================================================================
// Row-band repaint from already-fetched buffers
// ==============================================================================
//
// `render_pane`'s mismatch branch now consults `plan_pane_repaint` on a
// `SlotsReused` frame: a safe `Rows(spans)` plan clears + repaints only
// those pane-row bands from the buffers this same call already bulk-fetched
// (no second model query); an unsafe or over-cap plan still falls back to
// the unconditional whole-pane repaint these tests' `paint_skip` siblings
// above already prove is unchanged.

/// True when every `RectFill` recorded in `ops` lies entirely within
/// `band`'s pixel extent — the "did this repaint stay inside its row band"
/// assertion shared by several tests below.
fn all_rect_fills_within(
    ops: &[DrawOp],
    band: iron_canvas_core::geometry::pixel_rect::PixelRect,
) -> bool {
    ops.iter().all(|op| match op {
        DrawOp::RectFill { rect, .. } => {
            rect.top_left.y >= band.top_left.y
                && rect.top_left.y + rect.height <= band.top_left.y + band.height
        }
        _ => true,
    })
}

#[test]
fn row_band_repaint_paints_only_the_changed_row_band() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    // An interior row (not the pane's own first/last row) so there is a
    // real neighbour on both sides for the border-safety check to clear.
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

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let new_ops = &ops[ops_before..];

    assert!(!new_ops.is_empty(), "a changed row must emit paint ops");
    assert!(
        all_rect_fills_within(new_ops, band_rect),
        "a one-row safe change must not paint outside its own row band"
    );
}

#[test]
fn row_band_repaint_avoids_second_model_fetch_across_two_spans() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let row_a = pane_range.r1 + 1;
    // Gap of 2 rows between the changed rows so `ContentWork`'s row merge
    // cannot merge them into a single span — this must stay TWO bands.
    let row_b = pane_range.r1 + 4;
    assert!(
        row_b < pane_range.r2,
        "fixture needs two genuine interior rows"
    );
    m.set_cell(row_a, pane_range.c1, "changed-a");
    m.set_cell(row_b, pane_range.c1, "changed-b");

    let band_a = frame
        .range_rect(RCRange {
            r1: row_a,
            c1: pane_range.c1,
            r2: row_a,
            c2: pane_range.c2,
        })
        .expect("row_a band visible");
    let band_b = frame
        .range_rect(RCRange {
            r1: row_b,
            c1: pane_range.c1,
            r2: row_b,
            c2: pane_range.c2,
        })
        .expect("row_b band visible");

    m.reset_bulk_fetch_calls();
    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let new_ops = &ops[ops_before..];

    // The load-bearing claim: painting N (here 2) row bands from buffers
    // already fetched this frame costs exactly the ONE set of four bulk
    // calls `render_pane`'s own upfront fetch always makes — never a
    // second query per span.
    assert_eq!(
        m.bulk_fetch_calls(),
        4,
        "two spans emitted by one mismatch must not cost a second model fetch"
    );
    assert!(!new_ops.is_empty(), "two changed rows must emit paint ops");
    assert!(
        new_ops.iter().all(|op| match op {
            DrawOp::RectFill { rect, .. } => {
                let in_a = rect.top_left.y >= band_a.top_left.y
                    && rect.top_left.y + rect.height <= band_a.top_left.y + band_a.height;
                let in_b = rect.top_left.y >= band_b.top_left.y
                    && rect.top_left.y + rect.height <= band_b.top_left.y + band_b.height;
                in_a || in_b
            }
            _ => true,
        }),
        "two non-adjacent safe rows must paint exactly their two bands, nothing between them"
    );
}

#[test]
fn row_band_repaint_wires_decoration_only_change_to_its_row() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
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
    // Value and style are untouched — only the CF decoration changes.
    m.set_decoration(
        changed_row,
        pane_range.c1,
        CellDecoration::DataBar(DataBarSpec {
            fraction: 0.5,
            color: "#3366cc".to_string(),
        }),
    );

    let band_rect = frame
        .range_rect(RCRange {
            r1: changed_row,
            c1: pane_range.c1,
            r2: changed_row,
            c2: pane_range.c2,
        })
        .expect("changed row's band must be visible");

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let new_ops = &ops[ops_before..];

    assert!(
        !new_ops.is_empty(),
        "a decoration-only change must emit paint ops"
    );
    assert!(
        all_rect_fills_within(new_ops, band_rect),
        "a decoration-only change must repaint only its own row's band"
    );
}

#[test]
fn row_band_repaint_falls_back_to_full_on_border_unsafe_change() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let changed_row = pane_range.r1 + 2;
    // The row below must still be inside the pane for the shared-edge
    // boundary check to be the thing that trips Full (not the pane's own
    // outer edge, which needs no check).
    assert!(
        changed_row < pane_range.r2,
        "fixture needs a row below in-pane"
    );
    m.set_cell(changed_row, pane_range.c1, "changed");
    m.set_style(
        changed_row,
        pane_range.c1,
        CellStyle {
            border: Border {
                bottom: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );

    let pane_rect = frame.range_rect(pane_range).expect("pane rect visible");

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let first_new = ops.get(ops_before).expect("mismatch must paint something");

    assert!(
        matches!(first_new, DrawOp::RectFill { rect, .. } if *rect == pane_rect),
        "a new bottom border at an internal span boundary must force a whole-pane clear, \
         not a scoped row-band clear: got {first_new:?}"
    );
}

#[test]
fn row_band_repaint_falls_back_to_full_on_border_removed_unsafe_change() {
    // Mirrors `row_band_repaint_falls_back_to_full_on_border_unsafe_change`
    // exactly, but exercises the OTHER direction of the border-safety check:
    // a bottom border present in the `painted` tree that this frame's edit
    // removes, rather than one newly added in `scratch`. Both directions are
    // read by `span_has_unsafe_border` (old OR new tree showing risk at the
    // boundary trips `Full`), but only the added-border direction had a
    // dispatch-level (real `DrawOp`) proof before this test — the removed
    // direction was proven only at the pure-planner level
    // (`row_fingerprint_repaint.rs`'s
    // `planning_old_top_border_at_internal_boundary_selects_full_repaint`).
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let changed_row = pane_range.r1 + 2;
    // The row below must still be inside the pane for the shared-edge
    // boundary check to be the thing that trips Full (not the pane's own
    // outer edge, which needs no check).
    assert!(
        changed_row < pane_range.r2,
        "fixture needs a row below in-pane"
    );
    // Baseline paint: the border is present in the FIRST (`painted`) tree.
    m.set_style(
        changed_row,
        pane_range.c1,
        CellStyle {
            border: Border {
                bottom: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: None,
                }),
                ..Border::default()
            },
            ..CellStyle::default()
        },
    );
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    // This frame: edit the row's content AND remove its bottom border —
    // present in `painted`, absent in `scratch`.
    m.set_cell(changed_row, pane_range.c1, "changed");
    m.set_style(changed_row, pane_range.c1, CellStyle::default());

    let pane_rect = frame.range_rect(pane_range).expect("pane rect visible");

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let first_new = ops.get(ops_before).expect("mismatch must paint something");

    assert!(
        matches!(first_new, DrawOp::RectFill { rect, .. } if *rect == pane_rect),
        "removing a bottom border at an internal span boundary must force a whole-pane clear, \
         not a scoped row-band clear: got {first_new:?}"
    );
}

#[test]
fn row_band_repaint_falls_back_to_full_when_spans_exceed_cap() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    // A taller canvas so nine spread-out rows (gap 3, per
    // `row_fingerprint_repaint.rs`'s equivalent planner-level test) all
    // fit inside one pane's visible range.
    let mut frame = Chrome::next(None, &m, canvas_large(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let base = pane_range.r1 + 1;
    let changed_rows: Vec<i32> = [0, 3, 6, 9, 12, 15, 18, 21, 24]
        .into_iter()
        .map(|delta| base + delta)
        .collect();
    for &row in &changed_rows {
        assert!(
            row < pane_range.r2,
            "fixture requires all nine rows to fit inside the visible pane"
        );
        m.set_cell(row, pane_range.c1, "changed");
    }

    let pane_rect = frame.range_rect(pane_range).expect("pane rect visible");

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();
    let first_new = ops.get(ops_before).expect("mismatch must paint something");

    assert!(
        matches!(first_new, DrawOp::RectFill { rect, .. } if *rect == pane_rect),
        "nine disjoint spans exceed the merge cap; must fall back to a single whole-pane clear, \
         got {first_new:?}"
    );
}

// ==============================================================================
// Two mismatch-branch edge cases neither of the tests above happen to
// exercise.
// ==============================================================================

#[test]
fn row_band_repaint_never_painted_pane_under_slots_reuse_forces_full_repaint() {
    // No prior `render_pane` call at all on this pane: the `painted` tree
    // is still `PaneFingerprint::default()` (range `{0,0,0,0}`, digest `0`).
    // Promoting straight to `SlotsReused` before the very first paint routes
    // this pane through `plan_pane_repaint` (no real pane range is ever
    // `{0,0,0,0}`, so the digest check can't coincidentally match), which
    // must see `painted.range != scratch.range` and fall back to `Full`
    // rather than attempting a row-level walk against a tree with zero rows.
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    promote_to_slots_reuse(&mut frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let pane_rect = frame.range_rect(pane_range).expect("pane rect visible");

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    let ops = core.painter().ops();

    assert!(
        !ops.is_empty(),
        "a never-painted pane must still paint something on its first SlotsReused frame"
    );
    assert!(
        matches!(ops.first(), Some(DrawOp::RectFill { rect, .. }) if *rect == pane_rect),
        "a never-painted pane has no prior tree to diff row-for-row against — the mismatch \
         branch must fall back to a whole-pane clear, not a row-band attempt; got {:?}",
        ops.first()
    );
}

#[test]
fn strip_paint_then_unchanged_slots_reuse_frame_skips() {
    // `render_pane_strip` (the mechanism both the blit path and
    // `render_pane_damage` use) paints a strip WITHOUT committing a new
    // painted tree — so `painted` keeps the last full paint's range/digest.
    // Drive a damage-style strip repaint over an UNCHANGED row, then paint an
    // unchanged `SlotsReused` frame: `plan_pane_repaint` compares the freshly
    // rebuilt scratch against that still-valid `painted` tree, finds them
    // content-identical (same range, same digest), and lands on
    // `RepaintPlan::Skip`. This must paint nothing — proven purely through the
    // recorder's draw-op count.
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));

    // Prime the pane: a normal Fresh paint commits the `painted` tree.
    core.render_pane(&m, PaneRegion::BottomRight, &frame);

    let pane_range = PaneRegion::BottomRight
        .range(&frame)
        .expect("BottomRight must have a range on this canvas");
    let damaged_row = pane_range.r1 + 1;
    assert!(
        damaged_row <= pane_range.r2,
        "fixture needs a row inside the pane"
    );

    // Content is untouched — this only exercises the strip-splice machinery,
    // not a real edit.
    core.render_pane_damage(
        &m,
        &frame,
        PaneRegion::BottomRight,
        &[RowSpan {
            r1: damaged_row,
            r2: damaged_row,
        }],
    );

    promote_to_slots_reuse(&mut frame);

    let ops_before = core.painter().ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert_eq!(
        core.painter().ops().len(),
        ops_before,
        "an unchanged SlotsReuse frame after a strip paint must Skip via plan_pane_repaint's \
         digest-equal fast path, not repaint"
    );
}
