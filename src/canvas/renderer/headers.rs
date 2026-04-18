//! Headers (row numbers + column letters), corner box, and frozen-pane
//! separator strokes.
//!
//! The inline corner box + separator drawing used to live in `render()`.
//! Both are now `CanvasRenderer` methods so the main render loop reads as
//! a sequence of intent-revealing calls: `draw_frozen_separators(&frc)`,
//! `draw_corner_box()`, `render_row_headers(...)`, ... .

use ironcalc_base::UserModel;

use super::super::geometry::{
    col_name, col_width, row_height, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use super::super::types::FrozenRC;
use super::text::DEFAULT_FONT_FAMILY;
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

#[derive(Copy, Clone)]
enum HeaderAxis {
    Row,
    Column,
}
impl CanvasRenderer {
    /// Thick separator strokes between frozen bands and the scrollable grid.
    pub(super) fn draw_frozen_separators(&self, frc: &FrozenRC) {
        let ctx = &self.ctx;
        // `frc.offset.y = HEADER_ROW_HEIGHT + frozen_h + FROZEN_SEP` (when rows > 0),
        // so `sep_y = frc.offset.y - FROZEN_SEP + 0.5` gives the correct position.
        let sep_y = frc.offset.y - FROZEN_SEP + 0.5;
        let sep_x = frc.offset.x - FROZEN_SEP + 0.5;
        let half_sep = FROZEN_SEP / 2.0;

        if frc.rows > 0 {
            ctx.set_line_width(FROZEN_SEP);
            ctx.set_stroke_style_str(self.theme.grid_separator_color);
            ctx.begin_path();
            ctx.move_to(0.0, sep_y + half_sep);
            ctx.line_to(self.width, sep_y + half_sep);
            ctx.stroke();
            ctx.set_line_width(STANDARD_BORDER_WIDTH);
        }
        if frc.cols > 0 {
            ctx.set_line_width(FROZEN_SEP);
            ctx.set_stroke_style_str(self.theme.grid_separator_color);
            ctx.begin_path();
            ctx.move_to(sep_x + half_sep, 0.0);
            ctx.line_to(sep_x + half_sep, self.height);
            ctx.stroke();
            ctx.set_line_width(STANDARD_BORDER_WIDTH);
        }
    }

    /// Top-left blank square plus the two axis lines that separate the
    /// header strips from the cell area.
    pub(super) fn draw_corner_box(&self) {
        let ctx = &self.ctx;
        ctx.set_fill_style_str(self.theme.header_bg);
        ctx.fill_rect(0.0, 0.0, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT);
        ctx.set_stroke_style_str(self.theme.header_border_color);
        ctx.set_line_width(STANDARD_BORDER_WIDTH);
        ctx.begin_path();
        ctx.move_to(0.0, HEADER_ROW_HEIGHT + 0.5);
        ctx.line_to(self.width, HEADER_ROW_HEIGHT + 0.5);
        ctx.stroke();
        ctx.begin_path();
        ctx.move_to(HEADER_COL_WIDTH + 0.5, 0.0);
        ctx.line_to(HEADER_COL_WIDTH + 0.5, self.height);
        ctx.stroke();
    }

    pub(super) fn render_row_headers(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_rows: i32,
        frozen_y: f64,
    ) {
        let view = model.get_selected_view();
        let sel_start = view.range[0].min(view.range[2]);
        let sel_end = view.range[0].max(view.range[2]);

        self.ctx
            .set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen rows strip.
        let mut y = HEADER_ROW_HEIGHT + 0.5;
        for row in 1..=frozen_rows {
            let rh = row_height(model, sheet, row);
            if rh <= 0.0 {
                continue;
            }
            let selected = row >= sel_start && row <= sel_end;
            self.draw_header_cell(HeaderAxis::Row, row, y, rh, selected);
            y += rh;
        }

        // Scrollable rows strip.
        let mut y = if frozen_rows > 0 {
            frozen_y
        } else {
            HEADER_ROW_HEIGHT + 0.5
        };
        for row in self.vis.row_first..=self.vis.row_last {
            let rh = row_height(model, sheet, row);
            if rh <= 0.0 {
                continue;
            }
            let selected = row >= sel_start && row <= sel_end;
            self.draw_header_cell(HeaderAxis::Row, row, y, rh, selected);
            y += rh;
        }
    }

    pub(super) fn render_column_headers(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_cols: i32,
        frozen_x: f64,
    ) {
        let view = model.get_selected_view();
        let sel_start = view.range[1].min(view.range[3]);
        let sel_end = view.range[1].max(view.range[3]);

        self.ctx
            .set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen columns strip.
        let mut x = HEADER_COL_WIDTH + 0.5;
        for col in 1..=frozen_cols {
            let cw = col_width(model, sheet, col);
            if cw <= 0.0 {
                continue;
            }
            let selected = col >= sel_start && col <= sel_end;
            self.draw_header_cell(HeaderAxis::Column, col, x, cw, selected);
            x += cw;
        }

        // Scrollable columns strip.
        let mut x = if frozen_cols > 0 {
            frozen_x
        } else {
            HEADER_COL_WIDTH + 0.5
        };
        for col in self.vis.col_first..=self.vis.col_last {
            let cw = col_width(model, sheet, col);
            if cw <= 0.0 {
                continue;
            }
            let selected = col >= sel_start && col <= sel_end;
            self.draw_header_cell(HeaderAxis::Column, col, x, cw, selected);
            x += cw;
        }
    }

    /// Paint one header cell: border strip, body fill, and label.
    ///
    /// `along` is the cross-axis-fixed position (top_y for rows, x for cols);
    /// `thickness` is the cell's extent along the same axis (rh / cw).
    /// Both axes follow the identical three-step paint so only the rect
    /// orientations and label source differ.
    fn draw_header_cell(
        &self,
        axis: HeaderAxis,
        index: i32,
        along: f64,
        thickness: f64,
        selected: bool,
    ) {
        let ctx = &self.ctx;
        let body_bg = if selected {
            self.theme.header_selected_bg
        } else {
            self.theme.header_bg
        };
        let text_color = if selected {
            self.theme.header_selected_color
        } else {
            self.theme.header_text_color
        };

        ctx.set_fill_style_str(self.theme.header_border_color);
        match axis {
            HeaderAxis::Row => ctx.fill_rect(0.5, along, HEADER_COL_WIDTH, thickness),
            HeaderAxis::Column => ctx.fill_rect(along, 0.5, thickness, HEADER_ROW_HEIGHT),
        }

        ctx.set_fill_style_str(body_bg);
        match axis {
            HeaderAxis::Row => ctx.fill_rect(0.5, along + 0.5, HEADER_COL_WIDTH, thickness - 1.0),
            HeaderAxis::Column => {
                ctx.fill_rect(along + 0.5, 0.5, thickness - 1.0, HEADER_ROW_HEIGHT)
            }
        }

        ctx.set_fill_style_str(text_color);
        let (label, cx, cy) = match axis {
            HeaderAxis::Row => (
                index.to_string(),
                HEADER_COL_WIDTH / 2.0,
                along + thickness / 2.0,
            ),
            HeaderAxis::Column => (
                col_name(index),
                along + thickness / 2.0,
                HEADER_ROW_HEIGHT / 2.0,
            ),
        };
        ctx.fill_text(&label, cx, cy).ok();
    }
}
