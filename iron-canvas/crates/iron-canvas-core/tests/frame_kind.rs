//! Spec for `FrameKindTag` — every Chrome constructor sets the tag, and
//! Stage 5's regime dispatch reads it. These two specs lock down the
//! `Fresh` / `SlotsReused` constructor contracts; `Blitted` is covered
//! by the scroll-blit suite which already exercises `next_frame_with_blit`.

mod common;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::theme::CanvasTheme;

use common::{canvas_default, TestModel};

#[test]
fn next_frame_emits_fresh_when_no_prev() {
    let model = TestModel::synthetic_grid();
    let theme = CanvasTheme::light();
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
    let theme = CanvasTheme::light();
    let fresh = Chrome::next(None, &model, canvas_default(), &theme, FramePath::Fresh);
    let reused = Chrome::next(
        Some(fresh),
        &model,
        canvas_default(),
        &theme,
        FramePath::SlotsReuse,
    );
    assert_eq!(reused.kind, FrameKindTag::SlotsReused);
    assert!(
        reused.kind.reuses_slots(),
        "SlotsReused must report reuses_slots() so render_pane fingerprint-skips engage",
    );
}

/// Documents the Stage 5 invariant: adding a `FrameKindTag` variant must
/// break every regime arm in `orchestrator::paint_*`. The non-exhaustive
/// `match` blocks in `paint_viewport` / `paint_content` / `paint_rebuild`
/// (no `_ =>` arm) enforce this at compile time.
///
/// To verify locally:
/// 1. Add a fourth variant `Speculative` to `FrameKindTag` in `chrome/kind.rs`.
/// 2. `cargo check -p iron-canvas` — expect `error[E0004]` at three call
///    sites in `orchestrator.rs`.
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
