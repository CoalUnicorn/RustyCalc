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
        let ctx = &self.ctx;
        let view = model.get_selected_view();
        let sel_row_start = view.range[0].min(view.range[2]);
        let sel_row_end = view.range[0].max(view.range[2]);

        ctx.set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        let first_row = if frozen_rows == 0 {
            self.vis.row_first
        } else {
            1
        };
        let mut top_y = if first_row == 1 {
            HEADER_ROW_HEIGHT + 0.5
        } else {
            frozen_y
        };

        let mut row = first_row;
        loop {
            if row > self.vis.row_last {
                break;
            }
            let rh = row_height(model, sheet, row);
            if rh > 0.0 {
                let selected = row >= sel_row_start && row <= sel_row_end;
                ctx.set_fill_style_str(self.theme.header_border_color);
                ctx.fill_rect(0.5, top_y, HEADER_COL_WIDTH, rh);
                ctx.set_fill_style_str(if selected {
                    self.theme.header_selected_bg
                } else {
                    self.theme.header_bg
                });
                ctx.fill_rect(0.5, top_y + 0.5, HEADER_COL_WIDTH, rh - 1.0);
                ctx.set_fill_style_str(if selected {
                    self.theme.header_selected_color
                } else {
                    self.theme.header_text_color
                });
                ctx.fill_text(&row.to_string(), HEADER_COL_WIDTH / 2.0, top_y + rh / 2.0)
                    .ok();
                top_y += rh;
            }
            if row == frozen_rows {
                top_y = frozen_y;
                row = self.vis.row_first;
            } else {
                row += 1;
            }
        }
    }

    pub(super) fn render_column_headers(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_cols: i32,
        frozen_x: f64,
    ) {
        let ctx = &self.ctx;
        let view = model.get_selected_view();
        let sel_col_start = view.range[1].min(view.range[3]);
        let sel_col_end = view.range[1].max(view.range[3]);

        ctx.set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen columns strip.
        let mut x = HEADER_COL_WIDTH + 0.5;
        for col in 1..=frozen_cols {
            let cw = col_width(model, sheet, col);
            self.draw_col_header(col, x, cw, sel_col_start, sel_col_end);
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
            self.draw_col_header(col, x, cw, sel_col_start, sel_col_end);
            x += cw;
        }
    }

    fn draw_col_header(
        &self,
        col: i32,
        x: f64,
        cw: f64,
        sel_col_start: i32,
        sel_col_end: i32,
    ) {
        let ctx = &self.ctx;
        let selected = col >= sel_col_start && col <= sel_col_end;
        ctx.set_fill_style_str(self.theme.header_border_color);
        ctx.fill_rect(x, 0.5, cw, HEADER_ROW_HEIGHT);
        ctx.set_fill_style_str(if selected {
            self.theme.header_selected_bg
        } else {
            self.theme.header_bg
        });
        ctx.fill_rect(x + 0.5, 0.5, cw - 1.0, HEADER_ROW_HEIGHT);
        ctx.set_fill_style_str(if selected {
            self.theme.header_selected_color
        } else {
            self.theme.header_text_color
        });
        ctx.fill_text(&col_name(col), x + cw / 2.0, HEADER_ROW_HEIGHT / 2.0)
            .ok();
    }
}
