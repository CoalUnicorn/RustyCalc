#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic_in_result_fn)]

// Test fixture - a configurable in-memory CanvasModel.
//
// Only methods exercised by viewport / frozen-pane math are wired up.
// Style / cell-content methods stay `unimplemented!()` so a future test
// that touches them fails loudly rather than silently consuming defaults.

use crate::geometry::constants::{
    AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP,
    HEADER_COL_WIDTH, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
};
use crate::geometry::frame::frozen::FrozenRC;
use crate::geometry::frame::pixel_offset::PixelOffsets;
use crate::types::ui::HitTest;
use crate::{geometry::frame::FrameContext, CanvasView};
use crate::{CanvasModel, CanvasSize, PixelRect, Point, RCRange};

struct MockCanvasModel {
    sheet: u32,
    frozen_rows: i32,
    frozen_cols: i32,
    row_height: f64,
    col_width: f64,
    range: [i32; 4],
    top_row: i32,
    left_column: i32,
}

impl Default for MockCanvasModel {
    fn default() -> Self {
        Self {
            sheet: 0,
            frozen_rows: 0,
            frozen_cols: 0,
            row_height: DEFAULT_ROW_HEIGHT,
            col_width: DEFAULT_COL_WIDTH,
            range: [1, 1, 1, 1],
            top_row: 1,
            left_column: 1,
        }
    }
}

impl CanvasModel for MockCanvasModel {
    fn get_selected_sheet(&self) -> u32 {
        self.sheet
    }
    fn get_selected_view(&self) -> CanvasView {
        CanvasView {
            sheet: self.sheet,
            row: self.range[0],
            column: self.range[1],
            range: RCRange::from(self.range),
            top_row: self.top_row,
            left_column: self.left_column,
        }
    }
    fn get_frozen_rows_count(&self, _sheet: u32) -> Option<i32> {
        Some(self.frozen_rows)
    }
    fn get_frozen_columns_count(&self, _sheet: u32) -> Option<i32> {
        Some(self.frozen_cols)
    }
    fn get_row_height(&self, _sheet: u32, _row: i32) -> Option<f64> {
        Some(self.row_height)
    }
    fn get_column_width(&self, _sheet: u32, _column: i32) -> Option<f64> {
        Some(self.col_width)
    }
    fn get_show_grid_lines(&self, _sheet: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<ironcalc_base::types::Style> {
        None
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<ironcalc_base::types::CellType> {
        unimplemented!("cell type not used by these tests")
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        unimplemented!("cell value not used by these tests")
    }
}

// FrozenRC

#[test]
fn frozen_rc_no_freeze_has_no_bands_and_origin_skips_separator() {
    let m = MockCanvasModel::default();
    let frc = FrozenRC::from_model(&m);
    assert_eq!(frc.frozen_rows_count(), 0);
    assert_eq!(frc.frozen_cols_count(), 0);
    assert_eq!(frc.offset.x, HEADER_COL_WIDTH);
    assert_eq!(frc.offset.y, HEADER_ROW_HEIGHT);
}

#[test]
fn frozen_rc_rows_only_adds_separator_on_y_only() {
    let m = MockCanvasModel {
        frozen_rows: 2,
        ..Default::default()
    };
    let frc = FrozenRC::from_model(&m);
    assert_eq!(frc.frozen_rows_count(), 2);
    assert_eq!(frc.frozen_cols_count(), 0);
    assert_eq!(frc.offset.x, HEADER_COL_WIDTH);
    assert_eq!(
        frc.offset.y,
        HEADER_ROW_HEIGHT + 2.0 * DEFAULT_ROW_HEIGHT + FROZEN_SEP
    );
}

#[test]
fn frozen_rc_both_axes_add_separator_on_each() {
    let m = MockCanvasModel {
        frozen_rows: 1,
        frozen_cols: 3,
        ..Default::default()
    };
    let frc = FrozenRC::from_model(&m);
    assert_eq!(frc.frozen_rows_count(), 1);
    assert_eq!(frc.frozen_cols_count(), 3);
    assert_eq!(
        frc.offset.x,
        HEADER_COL_WIDTH + 3.0 * DEFAULT_COL_WIDTH + FROZEN_SEP
    );
    assert_eq!(
        frc.offset.y,
        HEADER_ROW_HEIGHT + DEFAULT_ROW_HEIGHT + FROZEN_SEP
    );
}

// PixelOffsets

#[test]
fn pixel_offsets_row_top_returns_zero_outside_precomputed_range() {
    let off = PixelOffsets {
        row_start: 10,
        row_tops: vec![0.0, 20.0, 40.0],
        col_start: 5,
        col_lefts: vec![0.0, 60.0],
        frozen_row_tops: vec![0.0],
        frozen_col_lefts: vec![0.0],
    };
    assert_eq!(off.row_top(10), 0.0);
    assert_eq!(off.row_top(11), 20.0);
    assert_eq!(off.row_top(99), 0.0);
    assert_eq!(off.col_left(5), 0.0);
    assert_eq!(off.col_left(6), 60.0);
    assert_eq!(off.col_left(99), 0.0);
}

// FrameContext: pixel ↔ cell math
//
// The frame is built fresh per test from the mock model and a canvas
// size large enough to make the test cells fall inside the visible
// region (so `cell_rect` returns Some). Queries are snapshot-only —
// every position lookup reads from the frame, never the model.

fn test_canvas() -> CanvasSize {
    CanvasSize {
        w: 1000.0,
        h: 800.0,
    }
}

#[test]
fn cell_rect_at_origin_starts_at_top_left_header_corner() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    let r = frame.cell_rect(1, 1).expect("origin cell is on screen");
    assert_eq!(r.top_left.x, HEADER_COL_WIDTH);
    assert_eq!(r.top_left.y, HEADER_ROW_HEIGHT);
    assert_eq!(r.width, DEFAULT_COL_WIDTH);
    assert_eq!(r.height, DEFAULT_ROW_HEIGHT);
}

#[test]
fn col_to_x_inside_frozen_band_skips_frozen_offset() {
    let m = MockCanvasModel {
        frozen_cols: 2,
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    assert_eq!(frame.col_to_x(1), HEADER_COL_WIDTH);
    assert_eq!(frame.col_to_x(2), HEADER_COL_WIDTH + DEFAULT_COL_WIDTH);
}

#[test]
fn col_to_x_past_frozen_seam_uses_frozen_offset_and_left_column() {
    let m = MockCanvasModel {
        frozen_cols: 2,
        left_column: 5,
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    let origin_x = frame.frozen.offset.x;
    // col 5 is the first scrollable on screen -> at the frozen offset
    assert_eq!(frame.col_to_x(5), origin_x);
    assert_eq!(frame.col_to_x(6), origin_x + DEFAULT_COL_WIDTH);
}

#[test]
fn autofill_handle_is_none_for_full_sheet_selection() {
    let m = MockCanvasModel {
        range: [1, 1, LAST_ROW, LAST_COLUMN],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    assert!(frame.autofill_handle().is_none());
}

#[test]
fn autofill_handle_lands_at_bottom_right_of_finite_selection() {
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    let p = frame
        .autofill_handle()
        .expect("finite selection has handle");
    assert_eq!(p.x, frame.col_to_x(5) + DEFAULT_COL_WIDTH);
    assert_eq!(p.y, frame.row_to_y(4) + DEFAULT_ROW_HEIGHT);
}

#[test]
fn autofill_handle_rect_anchors_at_bot_right_corner() {
    // Excel anchor: handle's top-left == selection's bottom-right corner,
    // so the handle visually pokes outside the selection rectangle.
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    let corner = frame.autofill_handle();
    let rect = frame.autofill_handle_rect();
    assert_eq!(rect.top_left.x, corner.unwrap().x - AUTOFILL_HANDLE_PX);
    assert_eq!(rect.top_left.y, corner.unwrap().y - AUTOFILL_HANDLE_PX);
    assert_eq!(rect.width, AUTOFILL_HANDLE_PX);
    assert_eq!(rect.height, AUTOFILL_HANDLE_PX);
}

#[test]
fn no_autofill_handle_rect_full_sheet_selection() {
    let m = MockCanvasModel {
        range: [1, 1, LAST_ROW, LAST_COLUMN],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    assert_eq!(
        frame.autofill_handle_rect(),
        PixelRect {
            top_left: Point { x: 0.0, y: 0.0 },
            width: 6.0,
            height: 6.0,
        }
    );
}

#[test]
fn hit_test_accepts_click_within_handle_pad() {
    // A click 1 px past the handle's bottom-right corner — inside the
    // 2-px forgiveness pad — must classify as AutofillHandle.
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    let rect = frame.autofill_handle_rect();
    let x = rect.right() + 1.0;
    let y = rect.bottom() + 1.0;
    match frame.hit_test(x, y) {
        HitTest::AutofillHandle { .. } => {}
        other => panic!("expected AutofillHandle within pad, got {:?}", other),
    }
}

#[test]
fn hit_test_rejects_click_past_handle_pad() {
    // One pixel past the pad on each axis — must fall through to Cell.
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    let rect = frame.autofill_handle_rect();
    let x = rect.right() + AUTOFILL_HIT_PAD_PX + 1.0;
    let y = rect.bottom() + AUTOFILL_HIT_PAD_PX + 1.0;
    match frame.hit_test(x, y) {
        HitTest::Cell { .. } => {}
        other => panic!("expected Cell past pad, got {:?}", other),
    }
}

#[test]
fn autofill_handle_tracks_in_place_selection_range_update() {
    // Mirrors the orchestrator's overlay-only repaint path: when the active
    // cell moves without scrolling, `paint_if_dirty` mutates the reused
    // frame's `selection_range` in place. The handle must land on the new
    // bottom-right, not the position captured by the previous full paint.
    let m = MockCanvasModel {
        range: [2, 3, 2, 3],
        ..Default::default()
    };
    let mut frame = FrameContext::current(&m, test_canvas());
    let before = frame.autofill_handle().expect("initial handle");

    frame.selection_range = RCRange {
        r1: 5,
        c1: 6,
        r2: 5,
        c2: 6,
    };
    let after = frame.autofill_handle().expect("post-update handle");

    assert_ne!(before, after, "handle must move with selection_range");
    assert_eq!(after.x, frame.col_to_x(6) + DEFAULT_COL_WIDTH);
    assert_eq!(after.y, frame.row_to_y(5) + DEFAULT_ROW_HEIGHT);
}

#[test]
fn cell_rect_off_screen_returns_none() {
    // Mock with default ~21px rows; canvas height 100 fits ~3 rows past
    // header, so row 50 is well past the visible region.
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, CanvasSize { w: 200.0, h: 100.0 });
    assert!(frame.cell_rect(50, 1).is_none());
}

#[test]
fn hit_test_corner() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    assert_eq!(frame.hit_test(5.0, 5.0), HitTest::Corner);
}

#[test]
fn hit_test_negative_is_outside() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    assert_eq!(frame.hit_test(-1.0, 10.0), HitTest::Outside);
    assert_eq!(frame.hit_test(10.0, -1.0), HitTest::Outside);
}

#[test]
fn hit_test_col_header_when_y_in_strip() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    // y inside header strip, x past row-header strip
    match frame.hit_test(HEADER_COL_WIDTH + 5.0, 5.0) {
        HitTest::ColHeader(c) => assert!(c >= 1),
        other => panic!("expected ColHeader, got {:?}", other),
    }
}

#[test]
fn hit_test_cell_in_grid() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    match frame.hit_test(HEADER_COL_WIDTH + 50.0, HEADER_ROW_HEIGHT + 50.0) {
        HitTest::Cell { row, column } => {
            assert!(row >= 1 && column >= 1);
        }
        other => panic!("expected Cell, got {:?}", other),
    }
}

#[test]
fn resize_handle_at_off_strip_is_none() {
    let m = MockCanvasModel::default();
    let frame = FrameContext::current(&m, test_canvas());
    // Inside cell grid -> no resize handle
    assert!(frame
        .resize_handle_at(HEADER_COL_WIDTH + 50.0, HEADER_ROW_HEIGHT + 50.0, 4.0)
        .is_none());
}

#[test]
fn pixel_to_col_round_trips_col_to_x() {
    // Round-trip the seam: col_to_x returns the LEFT edge of column c,
    // which is also the right edge of c-1. pixel_to_col on the left edge
    // resolves to c (strict-less-than break in the inner loop).
    let m = MockCanvasModel {
        frozen_cols: 2,
        left_column: 5,
        ..Default::default()
    };
    let frame = FrameContext::current(&m, test_canvas());
    for &c in &[1_i32, 2, 5, 6, 8] {
        let x = frame.col_to_x(c);
        // Nudge +0.5 to land safely inside the cell (avoid the edge).
        assert_eq!(frame.pixel_to_col(x + 0.5), c, "round-trip col {}", c);
    }
}
