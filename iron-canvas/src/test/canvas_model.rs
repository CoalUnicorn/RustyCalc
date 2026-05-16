#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

// Test fixture - a configurable in-memory CanvasModel.
//
// Only methods exercised by viewport / frozen-pane math are wired up.
// Style / cell-content methods stay `unimplemented!()` so a future test
// that touches them fails loudly rather than silently consuming defaults.

use crate::chrome::FramePath;
use crate::geometry::constants::{
    AUTOFILL_HANDLE_PX, AUTOFILL_HIT_PAD_PX, DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, FROZEN_SEP,
    HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT, LAST_COLUMN, LAST_ROW,
};
use crate::theme::LIGHT;
use crate::types::ui::HitTest;
use crate::{chrome::Chrome, CanvasView};
use crate::{CanvasModel, CanvasSize, RCRange};

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
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: self.sheet,
            row: self.range[0],
            column: self.range[1],
            selection: RCRange::from(self.range),
            top_row: self.top_row,
            left_column: self.left_column,
        })
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
        // Read by Chrome::build via ActiveCellSnapshot::capture; tests in
        // this module don't care about the value itself.
        None
    }
}

// Frozen-band geometry — exercised through the production path
// (Chrome::next -> PaneSet::build_rows/build_cols). After R7 the
// counts and offsets live on PaneSet directly; these tests pin the
// same math against the only path that reaches it in prod.

#[test]
fn no_freeze_has_no_bands_and_origin_skips_separator() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(
        None,
        &m,
        test_canvas(),
        &LIGHT,
        crate::chrome::FramePath::Fresh,
    );
    let p = &frame.pane_set;
    assert_eq!(p.frozen_rows_count(), 0);
    assert_eq!(p.frozen_cols_count(), 0);
    assert_eq!(p.frozen_offset_x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(p.frozen_offset_y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
}

#[test]
fn frozen_rows_only_adds_separator_on_y_only() {
    let m = MockCanvasModel {
        frozen_rows: 2,
        ..Default::default()
    };
    let frame = Chrome::next(
        None,
        &m,
        test_canvas(),
        &LIGHT,
        crate::chrome::FramePath::Fresh,
    );
    let p = &frame.pane_set;
    assert_eq!(p.frozen_rows_count(), 2);
    assert_eq!(p.frozen_cols_count(), 0);
    assert_eq!(p.frozen_offset_x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(
        p.frozen_offset_y,
        (f64::from(HEADER_ROW_HEIGHT + HEADER_OFFSET)
            + 2.0 * DEFAULT_ROW_HEIGHT
            + f64::from(FROZEN_SEP))
        .round() as i32
    );
}

#[test]
fn frozen_both_axes_add_separator_on_each() {
    let m = MockCanvasModel {
        frozen_rows: 1,
        frozen_cols: 3,
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let p = &frame.pane_set;
    assert_eq!(p.frozen_rows_count(), 1);
    assert_eq!(p.frozen_cols_count(), 3);
    assert_eq!(
        p.frozen_offset_x,
        (f64::from(HEADER_COL_WIDTH + HEADER_OFFSET)
            + 3.0 * DEFAULT_COL_WIDTH
            + f64::from(FROZEN_SEP))
        .round() as i32
    );
    assert_eq!(
        p.frozen_offset_y,
        (f64::from(HEADER_ROW_HEIGHT + HEADER_OFFSET) + DEFAULT_ROW_HEIGHT + f64::from(FROZEN_SEP))
            .round() as i32
    );
}

#[test]
fn frame_geometry_returns_zero_for_out_of_range_indices() {
    let frame = Chrome::next(
        None,
        &MockCanvasModel::default(),
        test_canvas(),
        &LIGHT,
        FramePath::Fresh,
    );
    let p = &frame.pane_set;
    assert_ne!(p.col_to_x(1), 0);
    assert_ne!(p.row_to_y(1), 0);
    assert_eq!(p.row_to_y(99999), 0);
    assert_eq!(p.col_to_x(99999), 0);
    assert_eq!(p.row_extent_at(99999), 0);
    assert_eq!(p.col_extent_at(99999), 0);
}

// Chrome: pixel ↔ cell math
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
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let r = frame.cell_rect(1, 1).expect("origin cell is on screen");
    assert_eq!(r.top_left.x, HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(r.top_left.y, HEADER_ROW_HEIGHT + HEADER_OFFSET);
    assert_eq!(f64::from(r.width), DEFAULT_COL_WIDTH);
    assert_eq!(f64::from(r.height), DEFAULT_ROW_HEIGHT);
}

#[test]
fn col_to_x_inside_frozen_band_skips_frozen_offset() {
    let m = MockCanvasModel {
        frozen_cols: 2,
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let p = &frame.pane_set;
    assert_eq!(p.col_to_x(1), HEADER_COL_WIDTH + HEADER_OFFSET);
    assert_eq!(
        p.col_to_x(2),
        (f64::from(HEADER_COL_WIDTH + HEADER_OFFSET) + DEFAULT_COL_WIDTH).round() as i32
    );
}

#[test]
fn col_to_x_past_frozen_seam_uses_frozen_offset_and_left_column() {
    let m = MockCanvasModel {
        frozen_cols: 2,
        left_column: 5,
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let p = &frame.pane_set;
    let origin_x = p.frozen_offset_x;
    // col 5 is the first scrollable on screen -> at the frozen offset
    assert_eq!(p.col_to_x(5), origin_x);
    assert_eq!(
        p.col_to_x(6),
        (f64::from(origin_x) + DEFAULT_COL_WIDTH).round() as i32
    );
}

#[test]
fn autofill_handle_is_none_for_full_sheet_selection() {
    let m = MockCanvasModel {
        range: [1, 1, LAST_ROW, LAST_COLUMN],
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    assert!(frame.autofill_handle().is_none());
}

#[test]
fn autofill_handle_lands_at_bottom_right_of_finite_selection() {
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let p = frame
        .autofill_handle()
        .expect("finite selection has handle");
    assert_eq!(
        p.x,
        (f64::from(frame.pane_set.col_to_x(5)) + DEFAULT_COL_WIDTH).round() as i32
    );
    assert_eq!(
        p.y,
        (f64::from(frame.pane_set.row_to_y(4)) + DEFAULT_ROW_HEIGHT).round() as i32
    );
}

#[test]
fn autofill_handle_rect_anchors_at_bot_right_corner() {
    // Excel anchor: handle's top-left == selection's bottom-right corner,
    // so the handle visually pokes outside the selection rectangle.
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let Some(corner) = frame.autofill_handle() else {
        panic!("expected autofill handle for partial-cell selection [2,3,4,5]");
    };
    let Some(rect) = frame.autofill_handle_rect() else {
        panic!("expected autofill rect for partial-cell selection [2,3,4,5]");
    };
    assert_eq!(rect.top_left.x, corner.x - AUTOFILL_HANDLE_PX);
    assert_eq!(rect.top_left.y, corner.y - AUTOFILL_HANDLE_PX);
    assert_eq!(rect.width, AUTOFILL_HANDLE_PX);
    assert_eq!(rect.height, AUTOFILL_HANDLE_PX);
}

#[test]
fn no_autofill_handle_rect_full_sheet_selection() {
    let m = MockCanvasModel {
        range: [1, 1, LAST_ROW, LAST_COLUMN],
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    assert!(frame.autofill_handle_rect().is_none());
}

#[test]
fn hit_test_accepts_click_within_handle_pad() {
    // A click 1 px past the handle's bottom-right corner — inside the
    // 2-px forgiveness pad — must classify as AutofillHandle.
    let m = MockCanvasModel {
        range: [2, 3, 4, 5],
        ..Default::default()
    };
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let Some(rect) = frame.autofill_handle_rect() else {
        panic!("expected autofill rect for partial-cell selection [2,3,4,5]");
    };
    let x = rect.right() + 1;
    let y = rect.bottom() + 1;
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
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let Some(rect) = frame.autofill_handle_rect() else {
        panic!("expected autofill rect for partial-cell selection [2,3,4,5]");
    };
    let x = rect.right() + AUTOFILL_HIT_PAD_PX + 1;
    let y = rect.bottom() + AUTOFILL_HIT_PAD_PX + 1;
    match frame.hit_test(x, y) {
        HitTest::Cell { .. } => {}
        other => panic!("expected Cell past pad, got {:?}", other),
    }
}

#[test]
fn autofill_handle_tracks_in_place_selection_range_update() {
    // Mirrors the orchestrator's overlay-only repaint path: when the active
    // cell moves without scrolling, `paintIfDirty` mutates the reused
    // frame's `selection_range` in place. The handle must land on the new
    // bottom-right, not the position captured by the previous full paint.
    let m = MockCanvasModel {
        range: [2, 3, 2, 3],
        ..Default::default()
    };
    let mut frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let before = frame.autofill_handle().expect("initial handle");

    frame.selection_range = RCRange {
        r1: 5,
        c1: 6,
        r2: 5,
        c2: 6,
    };
    let after = frame.autofill_handle().expect("post-update handle");

    assert_ne!(before, after, "handle must move with selection_range");
    assert_eq!(
        after.x,
        (f64::from(frame.pane_set.col_to_x(6)) + DEFAULT_COL_WIDTH).round() as i32
    );
    assert_eq!(
        after.y,
        (f64::from(frame.pane_set.row_to_y(5)) + DEFAULT_ROW_HEIGHT).round() as i32
    );
}

#[test]
fn cell_rect_off_screen_returns_none() {
    // Mock with default ~21px rows; canvas height 100 fits ~3 rows past
    // header, so row 50 is well past the visible region.
    let m = MockCanvasModel::default();
    let frame = Chrome::next(
        None,
        &m,
        CanvasSize { w: 200.0, h: 100.0 },
        &LIGHT,
        FramePath::Fresh,
    );
    assert!(frame.cell_rect(50, 1).is_none());
}

#[test]
fn hit_test_corner() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    assert_eq!(frame.hit_test(5, 5), HitTest::Corner);
}

#[test]
fn hit_test_negative_is_outside() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    assert_eq!(frame.hit_test(-1, 10), HitTest::Outside);
    assert_eq!(frame.hit_test(10, -1), HitTest::Outside);
}

#[test]
fn hit_test_col_header_when_y_in_strip() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    // y inside header strip, x past row-header strip
    match frame.hit_test(HEADER_COL_WIDTH + 5, 5) {
        HitTest::ColHeader(c) => assert!(c >= 1),
        other => panic!("expected ColHeader, got {:?}", other),
    }
}

#[test]
fn hit_test_cell_in_grid() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    match frame.hit_test(HEADER_COL_WIDTH + 50, HEADER_ROW_HEIGHT + 50) {
        HitTest::Cell { row, column } => {
            assert!(row >= 1 && column >= 1);
        }
        other => panic!("expected Cell, got {:?}", other),
    }
}

#[test]
fn resize_handle_at_off_strip_is_none() {
    let m = MockCanvasModel::default();
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    // Inside cell grid -> no resize handle
    assert!(frame
        .resize_handle_at(HEADER_COL_WIDTH + 50, HEADER_ROW_HEIGHT + 50, 4)
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
    let frame = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);
    let p = &frame.pane_set;
    for &c in &[1_i32, 2, 5, 6, 8] {
        let x = p.col_to_x(c);
        // Nudge +0.5 to land safely inside the cell (avoid the edge).
        assert_eq!(p.pixel_to_col(x + 1), Some(c), "round-trip col {}", c);
    }
}

// Slot-vec recycling — `rebuild` carries the four PaneSet Vec allocations
// from the outgoing frame into the new one, so steady-state rebuilds don't
// re-allocate. The strongest signal is pointer identity of the underlying
// buffers across the frame boundary.
#[test]
fn rebuild_recycles_pane_slot_buffers() {
    let m = MockCanvasModel {
        frozen_rows: 2,
        frozen_cols: 2,
        ..Default::default()
    };
    let f1 = Chrome::next(None, &m, test_canvas(), &LIGHT, FramePath::Fresh);

    let frozen_rows_ptr = f1.pane_set.frozen_rows.as_ptr();
    let scroll_rows_ptr = f1.pane_set.scroll_rows.as_ptr();
    let frozen_cols_ptr = f1.pane_set.frozen_cols.as_ptr();
    let scroll_cols_ptr = f1.pane_set.scroll_cols.as_ptr();
    let frozen_rows_cap = f1.pane_set.frozen_rows.capacity();
    let scroll_rows_cap = f1.pane_set.scroll_rows.capacity();
    let frozen_cols_cap = f1.pane_set.frozen_cols.capacity();
    let scroll_cols_cap = f1.pane_set.scroll_cols.capacity();

    let f2 = Chrome::next(Some(f1), &m, test_canvas(), &LIGHT, FramePath::Fresh);

    assert_eq!(f2.pane_set.frozen_rows.as_ptr(), frozen_rows_ptr);
    assert_eq!(f2.pane_set.scroll_rows.as_ptr(), scroll_rows_ptr);
    assert_eq!(f2.pane_set.frozen_cols.as_ptr(), frozen_cols_ptr);
    assert_eq!(f2.pane_set.scroll_cols.as_ptr(), scroll_cols_ptr);
    assert!(f2.pane_set.frozen_rows.capacity() >= frozen_rows_cap);
    assert!(f2.pane_set.scroll_rows.capacity() >= scroll_rows_cap);
    assert!(f2.pane_set.frozen_cols.capacity() >= frozen_cols_cap);
    assert!(f2.pane_set.scroll_cols.capacity() >= scroll_cols_cap);
}
