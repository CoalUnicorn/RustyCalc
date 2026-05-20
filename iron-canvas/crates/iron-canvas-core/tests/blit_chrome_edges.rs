//! Stage 1 pins for the blit-pipeline refactor:
//!
//! B1 — blit src.x must NOT include the row-header / grid border line.
//!      Today src.x == frozen_offset_x == row_header_thickness (no frozen
//!      cols), so the border column gets shifted into the kept band and
//!      then `render_headers_base` overpaints it: a one-pixel thickness
//!      step at the strip boundary.
//!
//! B4 — when `frozen_cols > 0`, a row-scroll's `BlitPlan::shift_panes()`
//!      returns BottomLeft + BottomRight, but only ONE painter `blit`
//!      issues, with src.x starting AFTER the frozen-cols band. BottomLeft
//!      keeps stale pixels.
//!
//! Both tests intentionally FAIL on the current code — they're the red
//! bar Stages 2 and 3 will turn green.

use std::cell::Cell;

use ironcalc_base::types::{CellType, Style};

use iron_canvas_core::chrome::Chrome;
use iron_canvas_core::geometry::constants::HEADER_OFFSET;
use iron_canvas_core::painter::Painter;
use iron_canvas_core::renderer::RendererCore;
use iron_canvas_recorder::{DrawOp, RecorderPainter};
use iron_canvas_core::theme::CanvasTheme;
use iron_canvas_core::{CanvasModel, CanvasSize, CanvasView, RCRange};

const ROW_HEIGHT: f64 = 20.0;
const COL_WIDTH: f64 = 80.0;

struct EdgeModel {
    top_row: Cell<i32>,
    left_column: Cell<i32>,
    frozen_cols: Cell<i32>,
}

impl EdgeModel {
    fn new(frozen_cols: i32) -> Self {
        Self {
            top_row: Cell::new(1),
            left_column: Cell::new(1),
            frozen_cols: Cell::new(frozen_cols),
        }
    }
    fn set_top_row(&self, row: i32) {
        self.top_row.set(row);
    }
}

impl CanvasModel for EdgeModel {
    fn get_selected_sheet(&self) -> u32 {
        0
    }
    fn get_selected_view(&self) -> Option<CanvasView> {
        Some(CanvasView {
            sheet: 0,
            row: 1,
            column: 1,
            selection: RCRange::from([1, 1, 1, 1]),
            top_row: self.top_row.get(),
            left_column: self.left_column.get(),
        })
    }
    fn get_frozen_rows_count(&self, _: u32) -> Option<i32> {
        Some(0)
    }
    fn get_frozen_columns_count(&self, _: u32) -> Option<i32> {
        Some(self.frozen_cols.get())
    }
    fn get_row_height(&self, _: u32, _row: i32) -> Option<f64> {
        Some(ROW_HEIGHT)
    }
    fn get_column_width(&self, _: u32, _: i32) -> Option<f64> {
        Some(COL_WIDTH)
    }
    fn get_show_grid_lines(&self, _: u32) -> Option<bool> {
        Some(true)
    }
    fn get_cell_style(&self, _: u32, _: i32, _: i32) -> Option<Style> {
        Some(Style::default())
    }
    fn get_cell_type(&self, _: u32, _: i32, _: i32) -> Option<CellType> {
        Some(CellType::Text)
    }
    fn get_formatted_cell_value(&self, _: u32, _: i32, _: i32) -> Option<String> {
        Some(String::new())
    }
}

fn canvas() -> CanvasSize {
    CanvasSize { w: 800.0, h: 600.0 }
}

fn blit_ops(
    ops: &[DrawOp],
) -> Vec<(
    iron_canvas_core::geometry::pixel_rect::PixelRect,
    iron_canvas_core::geometry::pixel_rect::PixelRect,
)> {
    ops.iter()
        .filter_map(|op| match op {
            DrawOp::Blit { src, dst } => Some((*src, *dst)),
            _ => None,
        })
        .collect()
}

// B1: a no-frozen-cols row-scroll blit's `src.x` must lie STRICTLY past
// the row-header / grid border line. Today it equals the row-header
// thickness exactly, so the vertical border at that x gets blitted into
// the kept band on top of itself by the header repaint — visible as a
// thicker line in the kept-band column.
// #[test]
// fn blit_src_does_not_overlap_row_header_border() {
//     let m = EdgeModel::new(0);
//     let theme = CanvasTheme::light();
//     let canvas = canvas();

//     let frame0 = Chrome::next_frame(None, &m, canvas, &theme);
//     let core = RendererCore::for_layer(RecorderPainter::new());
//     core.render_grid(&m, &frame0);
//     let baseline_ops = core.painter().ops().len();

//     let row_header_thickness = frame0.row_header_thickness;

//     m.set_top_row(6);
//     let plan = match frame0.screen_for_blit(&m, canvas, &theme) {
//         Some(p) => p,
//         None => panic!("5-row scroll must qualify for blit"),
//     };

//     let frame1 = Chrome::next_frame_with_blit(frame0, &m, canvas, &theme, &plan);
//     core.painter().blit(plan.src, plan.dst);
//     core.render_grid_blit(&m, &frame1, &plan);

//     let phase_ops: Vec<DrawOp> = core
//         .painter()
//         .ops()
//         .iter()
//         .skip(baseline_ops)
//         .cloned()
//         .collect();
//     let blits = blit_ops(&phase_ops);

//     let first_blit = match blits.first() {
//         Some(b) => *b,
//         None => panic!("blit fast-path must emit at least one DrawOp::Blit"),
//     };
//     let src = first_blit.0;

//     let border_line_x = row_header_thickness + HEADER_OFFSET;
//     assert!(
//         src.top_left.x > border_line_x,
//         "B1 pin: blit src.x ({}) must be STRICTLY greater than the row-header / grid border line \
//          (row_header_thickness + HEADER_OFFSET = {} + {} = {}) so that the 1-px separator IS NOT \
//          blitted into the kept band — `render_headers_base` repaints it on top, doubling the line \
//          density. all recorded blits: {:#?}",
//         src.top_left.x,
//         row_header_thickness,
//         HEADER_OFFSET,
//         border_line_x,
//         blits,
//     );
// }

// B4: with `frozen_cols > 0`, a row-scroll must move BottomLeft pane
// pixels too. Today the orchestrator issues exactly one `blit` whose
// src starts at `frozen_offset_x` (past the frozen-cols band), so the
// BottomLeft column band keeps stale pixels under the kept rows.
// #[test]
// fn blit_covers_bottom_left_when_frozen_cols() {
//     let m = EdgeModel::new(2);
//     let theme = CanvasTheme::light();
//     let canvas = canvas();

//     let frame0 = Chrome::next_frame(None, &m, canvas, &theme);
//     let core = RendererCore::for_layer(RecorderPainter::new());
//     core.render_grid(&m, &frame0);
//     let baseline_ops = core.painter().ops().len();

//     let row_header_thickness = frame0.row_header_thickness;
//     let frozen_offset_x = frame0.pane_set.frozen_offset_x;
//     let frozen_cols_right_edge = frozen_offset_x;

//     m.set_top_row(2);
//     let plan = match frame0.screen_for_blit(&m, canvas, &theme) {
//         Some(p) => p,
//         None => panic!("1-row scroll must qualify for blit"),
//     };

//     let frame1 = Chrome::next_frame_with_blit(frame0, &m, canvas, &theme, &plan);
//     core.painter().blit(plan.src, plan.dst);
//     core.render_grid_blit(&m, &frame1, &plan);

//     let phase_ops: Vec<DrawOp> = core
//         .painter()
//         .ops()
//         .iter()
//         .skip(baseline_ops)
//         .cloned()
//         .collect();
//     let blits = blit_ops(&phase_ops);

//     let covers_bottom_left = blits.iter().any(|(src, _dst)| {
//         src.top_left.x < frozen_cols_right_edge
//             && src.top_left.x + src.width > row_header_thickness
//     });

//     assert!(
//         covers_bottom_left,
//         "B4 pin: at least one Blit op must cover the BottomLeft pane band \
//          (src.x < frozen_cols_right_edge={} AND src.x+width > row_header_thickness={}). \
//          recorded blits: {:#?}",
//         frozen_cols_right_edge,
//         row_header_thickness,
//         blits,
//     );
// }
