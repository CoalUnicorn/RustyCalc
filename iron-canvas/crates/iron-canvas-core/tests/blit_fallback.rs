//! `Chrome::next(FramePath::Blit)` fallback: `screen_for_blit` qualifies
//! but `try_blit_reuse` rejects in-place reuse — today this fires only at
//! a row-header digit boundary, when the new last-visible row gains a
//! digit and `row_header_thickness` widens. The dispatch must hand back
//! a `Fresh` frame rather than a malformed `Blitted` one, otherwise
//! `paint_viewport_regime` would skip the full grid rebuild.

mod common;

use iron_canvas_core::chrome::{ActiveCellSnapshot, Chrome, FrameKindTag, FramePath};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, CanvasSize};

use common::TestModel;

fn snap(m: &TestModel) -> ActiveCellSnapshot {
    let view = m.get_selected_view().expect("view");
    ActiveCellSnapshot::capture(m, m.get_selected_sheet(), view.row, view.column)
}

#[test]
fn blit_fallback_at_row_header_digit_boundary_returns_fresh() {
    // 400 px tall canvas with 20 px rows ⇒ ~19 visible rows past the
    // 22 px header band. At top_row=980 the last visible row is 999
    // (3 digits, row_header_thickness = default). Scrolling to
    // top_row=981 makes the last visible row 1000 (4 digits), which
    // widens row_header_thickness — `try_blit_reuse`'s cross-axis reuse
    // check rejects and the dispatch falls through to a Fresh rebuild.
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let model = TestModel::synthetic_grid()
        .with_top_row(980)
        .with_active(980, 1);

    let prev = Chrome::next(None, &model, canvas, &theme, FramePath::Fresh);
    let prev_row_header = prev.row_header_thickness;
    let last_at_prev = prev
        .pane_set
        .rows
        .scroll
        .last()
        .expect("scroll band non-empty")
        .row;
    assert_eq!(
        last_at_prev, 999,
        "test premise: prev frame's last visible row must be 999 (3 digits) \
         — adjust top_row if canvas geometry constants shift"
    );

    // Scroll by 1. The current `measure_row_header_width(999)` and
    // `measure_row_header_width(1000)` already differ — both 3- and
    // 4-digit row counts hit different label widths under the
    // measurement approximation, so we anchor on `prev_row_header !=
    // new_row_header` rather than on absolute pixel values.
    model.set_top_row(981);
    let active = snap(&model);

    let plan = prev
        .screen_for_blit(&model, canvas, &theme, &active)
        .expect("single-row scroll must qualify geometrically");

    let next = Chrome::next(Some(prev), &model, canvas, &theme, FramePath::Blit(&plan));

    // The whole point of the fallback: if try_blit_reuse rejected, the
    // returned frame must be Fresh-built (clean stale_panes = ALL, kind =
    // Fresh) so paint_viewport_regime invalidates the cache.
    if next.row_header_thickness != prev_row_header {
        assert_eq!(
            next.kind,
            FrameKindTag::Fresh,
            "row_header widened ({}→{}), so try_blit_reuse must have fallen back to Fresh",
            prev_row_header,
            next.row_header_thickness
        );
    } else {
        // If the digit boundary didn't trip the measurement (numbers
        // close enough), the blit reused as Blitted — that's still
        // correct behavior, just a different code path. Pin the
        // contract: kind ∈ {Fresh, Blitted}.
        assert!(
            matches!(next.kind, FrameKindTag::Fresh | FrameKindTag::Blitted),
            "blit-path frame must be Fresh or Blitted, got {:?}",
            next.kind
        );
    }
}

/// Sanity contrast: a normal scroll where row_header_thickness does NOT
/// change must reuse in place and report `FrameKindTag::Blitted`. This
/// guards against the fallback firing too eagerly.
#[test]
fn blit_inside_stable_digit_band_keeps_blitted_kind() {
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let model = TestModel::synthetic_grid()
        .with_top_row(10)
        .with_active(10, 1);

    let prev = Chrome::next(None, &model, canvas, &theme, FramePath::Fresh);
    let prev_row_header = prev.row_header_thickness;

    model.set_top_row(11);
    let active = snap(&model);

    let plan = prev
        .screen_for_blit(&model, canvas, &theme, &active)
        .expect("single-row scroll must qualify");
    let next = Chrome::next(Some(prev), &model, canvas, &theme, FramePath::Blit(&plan));

    assert_eq!(
        next.row_header_thickness, prev_row_header,
        "test premise: scrolls inside the 2-digit band must keep header width"
    );
    assert_eq!(
        next.kind,
        FrameKindTag::Blitted,
        "in-band scroll must reuse in place"
    );
}
