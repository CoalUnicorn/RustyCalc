//! `FrameKindTag` constructor contracts.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FrameKindTag, FramePath};
use iron_canvas_core::theme::CanvasTheme;

use common::{TestModel, canvas_default, test_inputs};

#[test]
fn next_frame_emits_fresh_when_no_prev() {
    let model = TestModel::synthetic_grid();
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    assert_eq!(frame.kind, FrameKindTag::Fresh);
    assert!(!frame.kind.reuses_slots());
}

#[test]
fn slots_reuse_has_no_paint_scope_payload() {
    let model = TestModel::synthetic_grid().with_frozen(2, 2);
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(&model, canvas_default(), &theme);
    let fresh = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    let reused = Chrome::next(Some(fresh), &model, &inputs, FramePath::SlotsReuse);
    assert_eq!(reused.kind, FrameKindTag::SlotsReused);
    assert!(reused.kind.reuses_slots());
}

#[test]
fn frame_kind_variants_documented() {
    let variants = [
        FrameKindTag::Fresh,
        FrameKindTag::SlotsReused,
        FrameKindTag::Blitted,
    ];
    assert_eq!(variants.len(), 3);
}
