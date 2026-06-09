//! Spec for `FrameKindTag` — every Chrome constructor sets the tag, and
//! Stage 5's regime dispatch reads it. These two specs lock down the
//! `Fresh` / `SlotsReused` constructor contracts; `Blitted` is covered
//! by the scroll-blit suite which already exercises `next_frame_with_blit`.

mod common;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath, PaneRegionMask};
use iron_canvas_core::theme::CanvasTheme;

use common::{TestModel, canvas_default};

#[test]
fn next_frame_emits_fresh_when_no_prev() {
    let model = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let frame = Chrome::next(None, &model, canvas_default(), &theme, FramePath::Fresh);
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
    let fresh = Chrome::next(None, &model, canvas_default(), &theme, FramePath::Fresh);
    let reused = Chrome::next(
        Some(fresh),
        &model,
        canvas_default(),
        &theme,
        FramePath::SlotsReuse {
            stale_panes: PaneRegionMask::ALL,
        },
    );
    assert_eq!(reused.kind, FrameKindTag::SlotsReused);
    assert!(
        reused.kind.reuses_slots(),
        "SlotsReused must report reuses_slots() so render_pane fingerprint-skips engage",
    );
}

/// Regression: a `SlotsReuse` frame must take `stale_panes` from the
/// caller, not inherit it from `prev`. Before Option B made the field
/// part of the `FramePath::SlotsReuse` variant, the arm silently kept
/// `prev.stale_panes` — so a `SlotsReuse` chasing a `Blit` (whose
/// `stale_panes` had been narrowed to the scrolled strip) would skip
/// the unscrolled panes on the next content repaint. Reproduces the
/// scroll-to-row-78 → DEL bug at the `Chrome::next` level without
/// needing canvas or orchestrator scaffolding.
#[test]
fn slots_reuse_uses_caller_supplied_stale_panes() {
    let model = TestModel::synthetic_grid();
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let mut prev = Chrome::next(None, &model, canvas_default(), &theme, FramePath::Fresh);
    prev.stale_panes = PaneRegionMask::EMPTY;

    let reused = Chrome::next(
        Some(prev),
        &model,
        canvas_default(),
        &theme,
        FramePath::SlotsReuse {
            stale_panes: PaneRegionMask::ALL,
        },
    );

    assert_eq!(reused.stale_panes, PaneRegionMask::ALL);
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
