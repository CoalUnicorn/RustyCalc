#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

//! Behaviour-preservation harness for the Chrome refactor (Stage 1).
//!
//! Drives `RendererCore::<RecorderPainter>::render_grid` against a stub
//! model and asserts structural invariants on the recorded `DrawOp` log
//! that every Chrome sub-stage (1b–1e) must preserve. Stage 1a's
//! assertions are intentionally weak — they only prove the harness
//! compiles and runs against the current renderer. Stage 1e adds the
//! row-header widening assertions.

use ironcalc_base::types::{CellType, Style};

use crate::chrome::{measure_row_header_width, Chrome};
use crate::geometry::constants::{HEADER_COL_WIDTH, HEADER_OFFSET, HEADER_ROW_HEIGHT};
use crate::renderer::RendererCore;
use crate::test::painter::{DrawOp, RecorderPainter};
use crate::theme::CanvasTheme;
use crate::{CanvasModel, CanvasSize, CanvasView, RCRange};

struct StubModel {
    top_row: i32,
}

impl StubModel {
    fn at_top() -> Self {
        Self { top_row: 1 }
    }

    fn scrolled_to(top_row: i32) -> Self {
        Self { top_row }
    }
}

impl CanvasModel for StubModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: self.top_row,
            column: 1,
            selection: RCRange::from([self.top_row, 1, self.top_row, 1]),
            top_row: self.top_row,
            left_column: 1,
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_row_height(&self, _: u32, _: i32) -> Option<f64> {
        Some(20.0)
    }
    fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
        Some(80.0)
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Number)
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        Some(String::new())
    }
}

fn drive_render_grid(model: &StubModel, check: impl FnOnce(&Chrome, &[DrawOp])) {
    let theme = CanvasTheme::light();
    let canvas = CanvasSize { w: 600.0, h: 400.0 };
    let frame = Chrome::next_frame(None, model, canvas, &theme);
    let core = RendererCore::for_layer(RecorderPainter::new());
    core.render_grid(model, &frame);
    let ops = core.painter().ops();
    check(&frame, &ops);
}

// ─── Stage 1a / 1b / 1c / 1d carry-over — structural invariants ──────────

#[test]
fn render_grid_emits_draw_ops() {
    drive_render_grid(&StubModel::at_top(), |_, ops| {
        assert!(!ops.is_empty(), "render_grid must emit at least one DrawOp");
    });
}

#[test]
fn render_grid_brackets_grid_group_balanced() {
    drive_render_grid(&StubModel::at_top(), |_, ops| {
        let begins = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::BeginGroup { class: "grid" }))
            .count();
        let ends = ops
            .iter()
            .filter(|op| matches!(op, DrawOp::EndGroup))
            .count();
        assert_eq!(begins, 1, "exactly one BeginGroup(\"grid\") expected");
        assert_eq!(ends, 1, "exactly one EndGroup must pair the grid begin");
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
    drive_render_grid(&StubModel::at_top(), |frame_top, _| {
        drive_render_grid(&StubModel::scrolled_to(10_000), |frame_scrolled, _| {
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

    drive_render_grid(&StubModel::at_top(), |frame_top, ops_top| {
        let top_corner = find_corner_width(ops_top).expect("corner box at top must paint");
        assert_eq!(top_corner, frame_top.row_header_thickness);

        drive_render_grid(&StubModel::scrolled_to(1_000_000), |frame_far, ops_far| {
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
    drive_render_grid(&StubModel::at_top(), |frame, _| {
        assert_eq!(frame.col_header_thickness, HEADER_ROW_HEIGHT);
        assert_eq!(
            frame.cell_origin.x,
            frame.row_header_thickness + HEADER_OFFSET,
            "cell_origin.x must equal row_header_thickness + outer_offset"
        );
        assert_eq!(
            frame.cell_origin.y,
            frame.col_header_thickness + HEADER_OFFSET,
            "cell_origin.y must equal col_header_thickness + outer_offset"
        );
    });

    drive_render_grid(&StubModel::scrolled_to(1_000_000), |frame_far, _| {
        assert_eq!(
            frame_far.cell_origin.x,
            frame_far.row_header_thickness + HEADER_OFFSET,
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
    drive_render_grid(&StubModel::scrolled_to(1_000_000), |frame, _| {
        assert!(
            frame.cell_origin.x > HEADER_COL_WIDTH + HEADER_OFFSET,
            "test premise: 7-digit scroll must widen row header past default \
             (cell_origin.x={}, default={})",
            frame.cell_origin.x,
            HEADER_COL_WIDTH + HEADER_OFFSET,
        );
        let first_col = frame
            .pane_set
            .scroll_cols
            .first()
            .expect("scrolled view must emit at least one column slot");
        assert_eq!(
            first_col.left, frame.cell_origin.x,
            "first painted column slot must start at cell_origin.x — anything else \
             means headers misalign with cell columns"
        );
        let first_row = frame
            .pane_set
            .scroll_rows
            .first()
            .expect("scrolled view must emit at least one row slot");
        assert_eq!(
            first_row.top, frame.cell_origin.y,
            "first painted row slot must start at cell_origin.y"
        );
    });
}
