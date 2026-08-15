#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

//! Behaviour-preservation harness for the Chrome refactor (Stage 1).
//!
//! Drives `RendererCore::<RecorderPainter>::render_grid` against a stub
//! model and asserts structural invariants on the recorded `DrawOp` log
//! that every Chrome sub-stage (1b-1e) must preserve. Stage 1a's
//! assertions are intentionally weak — they only prove the harness
//! compiles and runs against the current renderer. Stage 1e adds the
//! row-header widening assertions.

mod common;

use iron_canvas_core::CanvasModel;
use iron_canvas_core::chrome::{Chrome, FramePath, measure_row_header_width};
use iron_canvas_core::geometry::constants::{CELL_AREA_INSET, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT};
use iron_canvas_core::painter::GroupClass;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_recorder::{DrawOp, RecorderPainter};

use common::{TestModel, canvas_default, test_inputs};

fn at_top() -> TestModel {
    TestModel::synthetic_grid()
}

fn scrolled_to(top_row: i32) -> TestModel {
    TestModel::synthetic_grid()
        .with_top_row(top_row)
        .with_active(top_row, 1)
}

fn drive_render_grid(model: &TestModel, check: impl FnOnce(&Chrome, &[DrawOp])) {
    let theme = std::rc::Rc::new(CanvasTheme::light());
    let inputs = test_inputs(model, canvas_default(), &theme);
    let frame = Chrome::next(None, model, &inputs, FramePath::Fresh);
    let core = RendererCore::for_layer(std::rc::Rc::new(RecorderPainter::new()));
    core.render_grid(model, &frame);
    let ops = core.painter().ops();
    check(&frame, &ops);
}

// ─── Stage 1a / 1b / 1c / 1d carry-over — structural invariants ──────────

#[test]
fn render_grid_emits_draw_ops() {
    drive_render_grid(&at_top(), |_, ops| {
        assert!(!ops.is_empty(), "render_grid must emit at least one DrawOp");
    });
}

#[test]
fn render_grid_brackets_grid_group_balanced() {
    drive_render_grid(&at_top(), |_, ops| {
        let count = |target: GroupClass| {
            ops.iter()
                .filter(|op| matches!(op, DrawOp::BeginGroup { class } if *class == target))
                .count()
        };
        assert_eq!(count(GroupClass::Grid), 1, "one Grid bracket expected");
        assert_eq!(count(GroupClass::Cells), 1, "one Cells bracket expected");
        assert_eq!(
            count(GroupClass::FrozenSep),
            1,
            "one FrozenSep bracket expected"
        );
        assert_eq!(
            count(GroupClass::Headers),
            1,
            "one Headers bracket expected"
        );
        assert_eq!(count(GroupClass::Corner), 1, "one Corner bracket expected");

        let begins = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::BeginGroup { .. }))
            .count();
        let ends = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::EndGroup))
            .count();
        assert_eq!(begins, ends, "begin/end groups must balance");
    });
}

// ─── Stage 1e — row_header_thickness widening ─────────────────────────────────

#[test]
fn measure_row_header_width_honors_default_minimum() {
    // Single-digit labels don't shrink the strip below the 30px historical
    // default. 3-digit labels may bump it slightly past the default thanks
    // to the pessimistic approximation, but stay close.
    assert_eq!(measure_row_header_width(1), HEADER_COL_WIDTH);
    let three_digit = measure_row_header_width(999);
    assert!(
        three_digit >= HEADER_COL_WIDTH,
        "3-digit width ({three_digit}) must clamp to default ({HEADER_COL_WIDTH})",
    );
    assert!(
        three_digit <= HEADER_COL_WIDTH + 8,
        "3-digit width ({three_digit}) must stay close to default ({HEADER_COL_WIDTH})",
    );
}

#[test]
fn measure_row_header_width_at_4_digits_widens() {
    let three = measure_row_header_width(999);
    let four = measure_row_header_width(9_999);
    assert!(
        four > three,
        "4-digit width ({four}) must exceed 3-digit width ({three})",
    );
}

#[test]
fn measure_row_header_width_at_7_digits_widens_further() {
    let four = measure_row_header_width(9_999);
    let seven = measure_row_header_width(1_048_576);
    assert!(
        seven > four,
        "7-digit width ({seven}) must exceed 4-digit width ({four})",
    );
}

#[test]
fn chrome_row_header_width_grows_when_scrolled_into_4_digits() {
    drive_render_grid(&at_top(), |frame_top, _| {
        drive_render_grid(&scrolled_to(10_000), |frame_scrolled, _| {
            assert_eq!(frame_top.row_header_thickness, HEADER_COL_WIDTH);
            assert!(
                frame_scrolled.row_header_thickness > frame_top.row_header_thickness,
                "scrolled chrome row_header_width ({}) must exceed top-of-sheet ({})",
                frame_scrolled.row_header_thickness,
                frame_top.row_header_thickness,
            );
        });
    });
}

#[test]
fn corner_box_rect_widens_when_scrolled_to_7_digit_rows() {
    // The corner box is the first RectFill emitted by render_grid that
    // sits at the canvas origin. Its width must follow chrome.row_header_thickness.
    let find_corner_width = |ops: &[DrawOp]| -> Option<i32> {
        ops.iter().find_map(|op| match op {
            DrawOp::RectFill { rect, .. } if rect.top_left.x == 0 && rect.top_left.y == 0 => {
                Some(rect.width)
            }
            _ => None,
        })
    };

    drive_render_grid(&at_top(), |frame_top, ops_top| {
        let top_corner = find_corner_width(ops_top).expect("corner box at top must paint");
        assert_eq!(top_corner, frame_top.row_header_thickness);

        drive_render_grid(&scrolled_to(1_000_000), |frame_far, ops_far| {
            let far_corner =
                find_corner_width(ops_far).expect("corner box scrolled-far must paint");
            assert_eq!(far_corner, frame_far.row_header_thickness);
            assert!(
                far_corner > top_corner,
                "scrolled corner-box width ({far_corner}) must exceed top corner ({top_corner})",
            );
        });
    });
}

/// R1 invariant: `cell_origin` is the single source of truth for where the
/// cell area begins. It must equal the two header thicknesses plus the
/// outer offset on each axis. Holds at every scroll position — `cell_origin.x`
/// tracks the dynamic `row_header_thickness`, `cell_origin.y` tracks the
/// (currently static) `col_header_thickness`.
#[test]
fn cell_origin_matches_header_thicknesses() {
    drive_render_grid(&at_top(), |frame, _| {
        assert_eq!(frame.col_header_thickness, HEADER_ROW_HEIGHT);
        assert_eq!(
            frame.cell_origin.x,
            frame.row_header_thickness + CELL_AREA_INSET,
            "cell_origin.x must equal row_header_thickness + outer_offset"
        );
        assert_eq!(
            frame.cell_origin.y,
            frame.col_header_thickness + CELL_AREA_INSET,
            "cell_origin.y must equal col_header_thickness + outer_offset"
        );
    });

    drive_render_grid(&scrolled_to(1_000_000), |frame_far, _| {
        assert_eq!(
            frame_far.cell_origin.x,
            frame_far.row_header_thickness + CELL_AREA_INSET,
            "cell_origin.x must track row_header_thickness even at deep scrolls"
        );
        assert_eq!(frame_far.col_header_thickness, HEADER_ROW_HEIGHT);
    });
}

/// R2 invariant — the bug fix. At a 7-digit scroll position the row-number
/// strip widens past `HEADER_COL_WIDTH`. Column headers must shift right by
/// the same amount, otherwise they desync from cell columns. The test pins
/// the first painted slot in each axis to the cell-area origin: any drift
/// here means headers and cells disagree on where column 1 / row 1 starts.
#[test]
fn column_headers_align_with_cell_columns_at_7_digit_scroll() {
    drive_render_grid(&scrolled_to(1_000_000), |frame, _| {
        assert!(
            frame.cell_origin.x > HEADER_COL_WIDTH + CELL_AREA_INSET,
            "test premise: 7-digit scroll must widen row header past default \
             (cell_origin.x={}, default={})",
            frame.cell_origin.x,
            HEADER_COL_WIDTH + CELL_AREA_INSET,
        );
        let first_col = frame
            .pane_set
            .cols
            .scroll
            .first()
            .expect("scrolled view must emit at least one column slot");
        assert_eq!(
            first_col.left, frame.cell_origin.x,
            "first painted column slot must start at cell_origin.x — anything else \
             means headers misalign with cell columns"
        );
        let first_row = frame
            .pane_set
            .rows
            .scroll
            .first()
            .expect("scrolled view must emit at least one row slot");
        assert_eq!(
            first_row.top, frame.cell_origin.y,
            "first painted row slot must start at cell_origin.y"
        );
    });
}

// ─── Stage 3 Task 2 — candidate construction consumes captured FrameInputs,
// not a live model read ───────────────────────────────────────────────────

/// `get_selected_sheet()` answers differently on its second call — standing
/// in for a live model that changed between `FrameInputs::capture` and
/// `Chrome::build` (or any other post-capture read). Every other accessor
/// forwards to an ordinary `TestModel`, so `FrameInputs::capture`'s own
/// sheet/view consistency check passes on the FIRST call, which is the only
/// one capture makes.
struct SheetChangesOnSecondRead {
    inner: TestModel,
    sheet_calls: std::cell::Cell<u32>,
}

impl SheetChangesOnSecondRead {
    fn new() -> Self {
        Self {
            inner: TestModel::synthetic_grid(),
            sheet_calls: std::cell::Cell::new(0),
        }
    }
}

impl iron_canvas_core::CellContentQuery for SheetChangesOnSecondRead {
    fn get_cell_style(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> iron_canvas_core::Fetched<iron_canvas_core::CellStyle> {
        self.inner.get_cell_style(sheet, row, column)
    }
    fn get_cell_type(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> iron_canvas_core::Fetched<iron_canvas_core::CellKind> {
        self.inner.get_cell_type(sheet, row, column)
    }
    fn get_formatted_cell_value(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> iron_canvas_core::Fetched<String> {
        self.inner.get_formatted_cell_value(sheet, row, column)
    }
}

impl iron_canvas_core::CanvasModel for SheetChangesOnSecondRead {
    fn get_selected_sheet(&self) -> Option<u32> {
        let call = self.sheet_calls.get();
        self.sheet_calls.set(call + 1);
        // First call (FrameInputs::capture) answers the model's real sheet,
        // matching `get_selected_view`'s embedded sheet so capture succeeds.
        // Every later call answers a different sheet — the value a
        // regressed, live-reading `Chrome::build` would wrongly pick up.
        if call == 0 { Some(0) } else { Some(7) }
    }
    fn get_selected_view(&self) -> Option<iron_canvas_core::CanvasView> {
        self.inner.get_selected_view()
    }
    fn get_frozen_rows_count(&self, sheet: u32) -> Option<i32> {
        self.inner.get_frozen_rows_count(sheet)
    }
    fn get_frozen_columns_count(&self, sheet: u32) -> Option<i32> {
        self.inner.get_frozen_columns_count(sheet)
    }
    fn get_row_height(&self, sheet: u32, row: i32) -> Option<f64> {
        self.inner.get_row_height(sheet, row)
    }
    fn get_column_width(&self, sheet: u32, column: i32) -> Option<f64> {
        self.inner.get_column_width(sheet, column)
    }
    fn get_show_grid_lines(&self, sheet: u32) -> Option<bool> {
        self.inner.get_show_grid_lines(sheet)
    }
    fn last_row(&self, sheet: u32) -> i32 {
        self.inner.last_row(sheet)
    }
    fn last_column(&self, sheet: u32) -> i32 {
        self.inner.last_column(sheet)
    }
}

#[test]
fn chrome_build_uses_captured_sheet_not_a_second_live_read() {
    let model = SheetChangesOnSecondRead::new();
    let theme = std::rc::Rc::new(CanvasTheme::light());

    // Capture is the one read `Chrome::build` is allowed to depend on.
    let inputs = iron_canvas_core::FrameInputs::capture(&model, canvas_default(), 1.0, theme, 0)
        .expect("healthy model must capture successfully");
    assert_eq!(inputs.sheet(), 0, "capture must see the first-call value");

    // Prove the premise: a second call to the same accessor really does
    // answer differently. If `Chrome::build` re-read the model instead of
    // consuming `inputs`, it would observe this value, not the captured one.
    assert_eq!(
        model.get_selected_sheet(),
        Some(7),
        "test premise: the model's second call must differ from its first"
    );

    let frame = Chrome::next(None, &model, &inputs, FramePath::Fresh);
    assert_eq!(
        frame.sheet, 0,
        "Chrome::build must use FrameInputs' captured sheet (0), not a fresh \
         model read that would observe the second-call value (7)"
    );
}
