//! Stage 1 fingerprint paint-skip — `render_pane` must emit zero `DrawOp`s
//! when the bulk-fetched buffers content-match the prior frame (under
//! `frame.kind == FrameKindTag::SlotsReused`), and must repaint exactly
//! the pane whose fingerprint changed.
//!
//! These tests target `render_pane` directly rather than `render_grid` so
//! the assertion surface stays the 4-pass per-pane walk. Header strips,
//! corner box, and frozen separators run above in `render_grid` and are
//! not fingerprint-gated.

mod common;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath, PaneRegion};
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::RecorderPainter;

use common::{TestModel, canvas_default};

fn paint_pane(model: &TestModel, frame: &Chrome, pane: PaneRegion) -> usize {
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_pane(model, pane, frame);
    let count = core.painter().ops().len();
    count
}

/// Mirrors the orchestrator's `SlotsReuse` branch: rotate the painted
/// fingerprints into `prev_pane_fingerprints` and flip the kind tag, so
/// the next `render_pane` call hits the skip-comparison branch.
fn promote_to_slots_reuse(frame: &mut Chrome) {
    frame.prev_pane_fingerprints = frame.pane_fingerprints.replace([0; 4]);
    frame.kind = FrameKindTag::SlotsReused;
}

#[test]
fn render_pane_skips_on_idempotent_repaint() {
    let m = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);

    // First paint runs through the full 4-pass walk; the kind is Fresh,
    // so the skip branch is gated off but `pane_fingerprints` is still
    // populated for the next frame.
    let first = paint_pane(&m, &frame, PaneRegion::BottomRight);
    assert!(first > 0, "first paint of a non-empty pane must emit ops");

    promote_to_slots_reuse(&mut frame);

    // Model unchanged ⇒ identical bulk-fetch buffers ⇒ identical
    // fingerprint ⇒ the entire 4-pass walk is skipped. Recorder log
    // must be byte-empty.
    let second = paint_pane(&m, &frame, PaneRegion::BottomRight);
    assert_eq!(
        second, 0,
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

    let _ = paint_pane(&m, &frame, PaneRegion::BottomLeft);
    let _ = paint_pane(&m, &frame, PaneRegion::BottomRight);

    promote_to_slots_reuse(&mut frame);

    // Col 5 lives past the frozen seam → BottomRight only.
    m.set_cell(1, 5, "changed");

    let bl_after = paint_pane(&m, &frame, PaneRegion::BottomLeft);
    let br_after = paint_pane(&m, &frame, PaneRegion::BottomRight);

    assert_eq!(
        bl_after, 0,
        "unaffected pane must skip — per-pane fingerprint is the load-bearing claim",
    );
    assert!(br_after > 0, "mutated pane must repaint");
}

#[test]
fn slots_reuse_holds_prior_pane_on_bridge_failure() {
    let m = TestModel::synthetic_grid();
    m.set_cell(1, 1, "still here");
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut frame = Chrome::next(None, &m, canvas_default(), &theme, FramePath::Fresh);
    let painter = std::rc::Rc::new(RecorderPainter::new());
    let core = RendererCore::for_layer(std::rc::Rc::clone(&painter));

    core.render_pane(&m, PaneRegion::BottomRight, &frame);
    assert!(
        !painter.ops().is_empty(),
        "first paint must populate prior pane pixels and cache"
    );
    promote_to_slots_reuse(&mut frame);

    m.set_value_bridge_fail(true);
    let before_failure = painter.ops().len();
    core.render_pane(&m, PaneRegion::BottomRight, &frame);

    assert_eq!(
        painter.ops().len(),
        before_failure,
        "BridgeFailed during SlotsReuse must hold prior pixels, not clear and repaint blank"
    );
}
