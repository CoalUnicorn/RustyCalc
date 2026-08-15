//! Piecewise address layout derived from one committed `Chrome` frame.

mod common;

use std::rc::Rc;

use iron_canvas_core::chrome::{Chrome, FramePath, GridLayout, PaneRegion};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasSize, RCRange};

use common::{TestModel, canvas_default, test_inputs};

fn frame(model: &TestModel, size: CanvasSize) -> Chrome {
    let theme = Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, size, &theme);
    Chrome::next(None, model, &inputs, FramePath::Fresh)
}

fn segments(layout: GridLayout) -> Vec<(PaneRegion, RCRange)> {
    layout
        .segments()
        .map(|segment| (segment.region(), segment.range()))
        .collect()
}

#[test]
fn layout_segments_deep_scroll() {
    let model = TestModel::synthetic_grid()
        .with_frozen_rows(2)
        .with_top_row(100);
    let frame = frame(&model, CanvasSize { w: 600.0, h: 468.0 });

    assert_eq!(
        segments(frame.grid_layout()),
        vec![
            (PaneRegion::TopRight, RCRange::from([1, 1, 2, 9])),
            (PaneRegion::BottomRight, RCRange::from([100, 1, 120, 9]),),
        ],
        "the layout must not bridge the scrolled-off rows 3..=99",
    );
}

#[test]
fn layout_unfrozen_single_segment() {
    let frame = frame(&TestModel::synthetic_grid(), canvas_default());
    let layout = frame.grid_layout();

    assert_eq!(layout.shape().row_lens(), [0, 20]);
    assert_eq!(layout.shape().col_lens(), [0, 9]);
    assert_eq!(layout.shape().frozen_rows(), 0);
    assert_eq!(layout.shape().frozen_cols(), 0);
    assert_eq!(
        segments(layout),
        vec![(PaneRegion::BottomRight, RCRange::from([1, 1, 20, 9]),)],
    );
}

#[test]
fn layout_both_axes_four_segments() {
    let model = TestModel::synthetic_grid()
        .with_frozen(2, 2)
        .with_top_row(100)
        .with_left_column(50);
    let frame = frame(&model, CanvasSize { w: 600.0, h: 468.0 });
    let layout = frame.grid_layout();

    assert_eq!(layout.shape().row_lens(), [2, 21]);
    assert_eq!(layout.shape().col_lens(), [2, 7]);
    assert_eq!(layout.shape().frozen_rows(), 2);
    assert_eq!(layout.shape().frozen_cols(), 2);
    assert_eq!(
        segments(layout),
        vec![
            (PaneRegion::TopLeft, RCRange::from([1, 1, 2, 2])),
            (PaneRegion::TopRight, RCRange::from([1, 50, 2, 56])),
            (PaneRegion::BottomLeft, RCRange::from([100, 1, 120, 2])),
            (PaneRegion::BottomRight, RCRange::from([100, 50, 120, 56]),),
        ],
    );
}

#[test]
fn layout_single_axis_two_segments() {
    let frozen_rows = TestModel::synthetic_grid()
        .with_frozen_rows(2)
        .with_top_row(100);
    let row_frame = frame(&frozen_rows, CanvasSize { w: 600.0, h: 468.0 });
    assert_eq!(
        segments(row_frame.grid_layout()),
        vec![
            (PaneRegion::TopRight, RCRange::from([1, 1, 2, 9])),
            (PaneRegion::BottomRight, RCRange::from([100, 1, 120, 9]),),
        ],
    );

    let frozen_cols = TestModel::synthetic_grid()
        .with_frozen_cols(2)
        .with_left_column(50);
    let col_frame = frame(&frozen_cols, canvas_default());
    assert_eq!(
        segments(col_frame.grid_layout()),
        vec![
            (PaneRegion::BottomLeft, RCRange::from([1, 1, 20, 2])),
            (PaneRegion::BottomRight, RCRange::from([1, 50, 20, 56]),),
        ],
    );
}

#[test]
fn quadrant_walk_order() {
    let all_regions = TestModel::synthetic_grid().with_frozen(1, 1);
    let all_frame = frame(&all_regions, canvas_default());
    assert_eq!(
        all_frame
            .grid_layout()
            .segments()
            .map(|segment| segment.region())
            .collect::<Vec<_>>(),
        vec![
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ],
    );

    let empty_left_band = TestModel::synthetic_grid().with_frozen_rows(1);
    let empty_frame = frame(&empty_left_band, canvas_default());
    assert_eq!(
        empty_frame
            .grid_layout()
            .segments()
            .map(|segment| segment.region())
            .collect::<Vec<_>>(),
        vec![PaneRegion::TopRight, PaneRegion::BottomRight],
        "empty TL and BL entries must be skipped without disturbing order",
    );
}
