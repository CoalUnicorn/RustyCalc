//! Headers (row numbers + column letters), corner box, and frozen-pane
//! separator strokes.
//!
//! The inline corner box + separator drawing used to live in `render()`.
//! Both are now `RendererCore` methods so the main render loop reads as
//! a sequence of intent-revealing calls: `draw_frozen_separators(&frc)`,
//! `draw_corner_box()`, `render_row_headers(...)`, ... .

use std::fmt::Write as _;

use crate::geometry::constants::{HEADER_OFFSET, STANDARD_BORDER_WIDTH};
use crate::geometry::frame::FrameContext;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{Axis, Point, Span};

use super::super::geometry::constants::{FROZEN_SEP, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT};

use super::RendererCore;

const HEADER_FONT: &str = "bold 12px Inter, Arial, sans-serif";

impl RendererCore {
    /// Thick separator strokes between frozen bands and the scrollable grid.
    pub(super) fn draw_frozen_separators(&self, frame: &FrameContext) {
        let frc = &frame.frozen;
        if frc.rows == 0 && frc.cols == 0 {
            return;
        }
        self.set_stroke_static(frame.theme.grid_separator_color);

        let sep_y = frc.offset.y - FROZEN_SEP / 2 + HEADER_OFFSET;
        let sep_x = frc.offset.x - FROZEN_SEP / 2 + HEADER_OFFSET;
        let canvas_w = frame.canvas_size.w as i32;
        let canvas_h = frame.canvas_size.h as i32;

        self.with_stroke_width(FROZEN_SEP, |this| {
            if frc.rows > 0 {
                this.stroke_hline(
                    Span {
                        from: 0,
                        to: canvas_w,
                    },
                    f64::from(sep_y),
                );
            }
            if frc.cols > 0 {
                this.stroke_vline(
                    f64::from(sep_x),
                    Span {
                        from: 0,
                        to: canvas_h,
                    },
                );
            }
        });
    }

    /// Top-left blank square plus the two axis lines that separate the
    /// header strips from the cell area.
    pub(super) fn draw_corner_box(&self, frame: &FrameContext) {
        let corner = PixelRect {
            top_left: Point { x: 0, y: 0 },
            width: HEADER_COL_WIDTH,
            height: HEADER_ROW_HEIGHT,
        };
        self.rect_fill(corner, frame.theme.header_bg);

        self.set_stroke_static(frame.theme.header_border_color);
        self.set_line_width_cached(STANDARD_BORDER_WIDTH);
        self.stroke_hline(
            Span {
                from: 0,
                to: frame.canvas_size.w as i32,
            },
            f64::from(HEADER_ROW_HEIGHT + HEADER_OFFSET),
        );
        self.stroke_vline(
            f64::from(HEADER_COL_WIDTH + HEADER_OFFSET),
            Span {
                from: 0,
                to: frame.canvas_size.h as i32,
            },
        );
    }

    /// Paint one header strip along `axis` with no selection highlighting.
    pub(super) fn render_headers_base(&self, axis: Axis, frame: &FrameContext) {
        self.set_font_static(HEADER_FONT);
        self.walk_header_strip(axis, frame, |this, i, along, t| {
            this.draw_header_cell(axis, frame, i, along, t, false);
        });
    }

    /// Overlay pass repainting only selected header cells so the grid layer
    /// never redraws on navigation — the base pass beneath stays intact.
    pub(super) fn render_header_highlights(&self, axis: Axis, frame: &FrameContext) {
        self.set_font_static(HEADER_FONT);
        let (sel_start, sel_end) = axis.selection_range(frame.selection_range);
        self.walk_header_strip(axis, frame, |this, i, along, t| {
            if i >= sel_start && i <= sel_end {
                this.draw_header_cell(axis, frame, i, along, t, true);
            }
        });
    }

    /// Shared scaffold: walk the frozen band (if any) then the visible band,
    /// reading extents from the frame's prefix-sum snapshot — zero model access.
    fn walk_header_strip(
        &self,
        axis: Axis,
        frame: &FrameContext,
        mut visit: impl FnMut(&Self, i32, i32, i32),
    ) {
        let frozen_count = axis.frozen_count(frame);
        let frozen_origin = axis.frozen_origin(frame);

        let mut frozen_cursor = axis.strip_start();
        for i in 1..=frozen_count {
            let t = axis.frame_extent(frame, i);
            if t <= 0 {
                continue;
            }
            visit(self, i, frozen_cursor, t);
            frozen_cursor += t;
        }

        let mut scroll_cursor = if frozen_count > 0 {
            frozen_origin
        } else {
            axis.strip_start()
        };
        for i in axis.visible_band(&frame.vis) {
            let t = axis.frame_extent(frame, i);
            if t <= 0 {
                continue;
            }
            visit(self, i, scroll_cursor, t);
            scroll_cursor += t;
        }
    }

    /// Paint a single header cell: border strip, body fill, and label.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for cols);
    /// `thickness` is the cell's extent along the same axis (rh / cw).
    fn draw_header_cell(
        &self,
        axis: Axis,
        frame: &FrameContext,
        index: i32,
        along: i32,
        thickness: i32,
        selected: bool,
    ) {
        let body_bg = if selected {
            frame.theme.header_selected_bg
        } else {
            frame.theme.header_bg
        };
        let text_color = if selected {
            frame.theme.header_selected_color
        } else {
            frame.theme.header_text_color
        };

        let full = axis.header_rect(along, thickness);
        // 1px inset on the cross-axis leaves the border strip visible top+bottom (row)
        // or left+right (column).
        let body = match axis {
            Axis::Row => full.inset(0, 1),
            Axis::Column => full.inset(1, 0),
        };

        self.rect_fill(full, frame.theme.header_border_color);
        self.rect_fill(body, body_bg);

        self.set_fill_static(text_color);
        let center = full.center();
        let snap_x = f64::from(center.x);
        let snap_y = f64::from(center.y);
        // Row labels: write the integer into the renderer-owned scratch String
        // (zero-alloc steady-state). Column labels: pull from the per-renderer
        // intern (one alloc per unique column over the renderer's lifetime).
        match axis {
            Axis::Row => {
                let mut buf = self.frame_cache.label_buf.borrow_mut();
                buf.clear();
                let _ = write!(&mut *buf, "{}", index);
                self.ctx.fill_text(&buf, snap_x, snap_y).ok();
            }
            Axis::Column => {
                let label = self.col_intern.get(index);
                self.ctx.fill_text(&label, snap_x, snap_y).ok();
            }
        }
    }
}
