//! Headers (row numbers + column letters), corner box, and frozen-pane
//! separator strokes.
//!
//! The inline corner box + separator drawing used to live in `render()`.
//! Both are now `CanvasRenderer` methods so the main render loop reads as
//! a sequence of intent-revealing calls: `draw_frozen_separators(&frc)`,
//! `draw_corner_box()`, `render_row_headers(...)`, ... .

use std::ops::RangeInclusive;

use ironcalc_base::UserModel;

use super::super::geometry::{
    col_name, col_width, row_height, FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use super::super::types::{Axis, FrozenRC};
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

        if frc.row_band.is_some() {
            ctx.set_line_width(FROZEN_SEP);
            ctx.set_stroke_style_str(self.theme.grid_separator_color);
            ctx.begin_path();
            ctx.move_to(0.0, sep_y + half_sep);
            ctx.line_to(self.width, sep_y + half_sep);
            ctx.stroke();
            ctx.set_line_width(STANDARD_BORDER_WIDTH);
        }
        if frc.col_band.is_some() {
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
        frozen_band: Option<&RangeInclusive<i32>>,
        frozen_y: f64,
    ) {
        let view = model.get_selected_view();
        let sel_start = view.range[0].min(view.range[2]);
        let sel_end = view.range[0].max(view.range[2]);

        self.ctx
            .set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen rows strip.
        let mut y = HEADER_ROW_HEIGHT + 0.5;
        if let Some(band) = frozen_band {
            for row in band.clone() {
                let rh = row_height(model, sheet, row);
                if rh <= 0.0 {
                    continue;
                }
                let selected = row >= sel_start && row <= sel_end;
                self.draw_header_cell(Axis::Row, row, y, rh, selected);
                y += rh;
            }
        }

        // Scrollable rows strip.
        let mut y = if frozen_band.is_some() {
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
            self.draw_header_cell(Axis::Row, row, y, rh, selected);
            y += rh;
        }
    }

    pub(super) fn render_column_headers(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_band: Option<&RangeInclusive<i32>>,
        frozen_x: f64,
    ) {
        let view = model.get_selected_view();
        let sel_start = view.range[1].min(view.range[3]);
        let sel_end = view.range[1].max(view.range[3]);

        self.ctx
            .set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen columns strip.
        let mut x = HEADER_COL_WIDTH + 0.5;
        if let Some(band) = frozen_band {
            for col in band.clone() {
                let cw = col_width(model, sheet, col);
                if cw <= 0.0 {
                    continue;
                }
                let selected = col >= sel_start && col <= sel_end;
                self.draw_header_cell(Axis::Column, col, x, cw, selected);
                x += cw;
            }
        }

        // Scrollable columns strip.
        let mut x = if frozen_band.is_some() {
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
            self.draw_header_cell(Axis::Column, col, x, cw, selected);
            x += cw;
        }
    }

    /// Paint one header cell: border strip, body fill, and label.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for cols);
    /// `thickness` is the cell's extent along the same axis (rh / cw).
    fn draw_header_cell(&self, axis: Axis, index: i32, along: f64, thickness: f64, selected: bool) {
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

        let full = axis.header_rect(along, thickness);
        // 1px inset on the cross-axis leaves the border strip visible top+bottom (row)
        // or left+right (column).
        let body = match axis {
            Axis::Row => full.inset(0.0, 0.5),
            Axis::Column => full.inset(0.5, 0.0),
        };

        self.rect_fill(full, self.theme.header_border_color);
        self.rect_fill(body, body_bg);

        self.ctx.set_fill_style_str(text_color);
        let center = full.center();
        let label = match axis {
            Axis::Row => index.to_string(),
            Axis::Column => col_name(index),
        };
        self.ctx.fill_text(&label, center.x, center.y).ok();
    }
}
