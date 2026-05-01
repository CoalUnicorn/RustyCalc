//! Headers (row numbers + column letters), corner box, and frozen-pane
//! separator strokes.
//!
//! The inline corner box + separator drawing used to live in `render()`.
//! Both are now `CanvasRenderer` methods so the main render loop reads as
//! a sequence of intent-revealing calls: `draw_frozen_separators(&frc)`,
//! `draw_corner_box()`, `render_row_headers(...)`, ... .

use std::ops::RangeInclusive;

use crate::{CanvasModel, Point, Span, HEADER_OFFSET};

use super::super::geometry::{
    col_name, Axis, FrozenRC, PixelRect, VisibleRegion, FROZEN_SEP, HEADER_COL_WIDTH,
    HEADER_ROW_HEIGHT,
};

use super::text::DEFAULT_FONT_FAMILY;
use super::{CanvasRenderer, STANDARD_BORDER_WIDTH};

impl CanvasRenderer {
    /// Thick separator strokes between frozen bands and the scrollable grid.
    pub(super) fn draw_frozen_separators(&self, frc: &FrozenRC) {
        if frc.row_band.is_none() && frc.col_band.is_none() {
            return;
        }
        self.set_stroke_static(self.theme.grid_separator_color);

        let sep_y = frc.offset.y - FROZEN_SEP / 2.0 + HEADER_OFFSET;
        let sep_x = frc.offset.x - FROZEN_SEP / 2.0 + HEADER_OFFSET;

        self.with_stroke_width(FROZEN_SEP, |this| {
            if frc.row_band.is_some() {
                this.stroke_hline(
                    Span {
                        from: 0.0,
                        to: this.width,
                    },
                    sep_y,
                );
            }
            if frc.col_band.is_some() {
                this.stroke_vline(
                    sep_x,
                    Span {
                        from: 0.0,
                        to: this.height,
                    },
                );
            }
        });
    }

    /// Top-left blank square plus the two axis lines that separate the
    /// header strips from the cell area.
    pub(super) fn draw_corner_box(&self) {
        let corner = PixelRect {
            top_left: Point { x: 0.0, y: 0.0 },
            width: HEADER_COL_WIDTH,
            height: HEADER_ROW_HEIGHT,
        };
        self.rect_fill(corner, self.theme.header_bg);

        self.set_stroke_static(self.theme.header_border_color);
        self.set_line_width_cached(STANDARD_BORDER_WIDTH);
        self.stroke_hline(
            Span {
                from: 0.0,
                to: self.width,
            },
            HEADER_ROW_HEIGHT + HEADER_OFFSET,
        );
        self.stroke_vline(
            HEADER_COL_WIDTH + HEADER_OFFSET,
            Span {
                from: 0.0,
                to: self.height,
            },
        );
    }

    /// Paint one header strip along `axis` with no selection highlighting.
    /// Selected indices are tracked for highlight rendering via `render_header_highlights`.
    pub(super) fn render_headers_base(
        &self,
        model: &dyn CanvasModel,
        axis: Axis,
        vis: &VisibleRegion,
        frozen_band: Option<&RangeInclusive<i32>>,
        frozen_origin: f64,
    ) {
        self.set_font_cached(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));
        self.walk_header_strip(model, axis, vis, frozen_band, frozen_origin, |this, i, along, t| {
            this.draw_header_cell(axis, i, along, t, false);
        });
    }

    /// Overlay pass repainting only selected header cells so the grid layer
    /// never redraws on navigation — the base pass beneath stays intact.
    pub(super) fn render_header_highlights(
        &self,
        model: &dyn CanvasModel,
        axis: Axis,
        vis: &VisibleRegion,
        frozen_band: Option<&RangeInclusive<i32>>,
        frozen_origin: f64,
    ) {
        let view = model.get_selected_view();
        let (sel_start, sel_end) = axis.selection_range(&view.range);

        self.walk_header_strip(model, axis, vis, frozen_band, frozen_origin, |this, i, along, t| {
            if i >= sel_start && i <= sel_end {
                this.draw_header_cell(axis, i, along, t, true);
            }
        });
    }

    /// Shared scaffold: walk the frozen band (if any) then the visible band,
    /// threading per-cell extents through a moving cursor along `axis`.
    /// `visit` receives `(self, index, along, thickness)` per non-collapsed
    /// cell — base/highlight passes diverge only in what they paint.
    fn walk_header_strip(
        &self,
        model: &dyn CanvasModel,
        axis: Axis,
        vis: &VisibleRegion,
        frozen_band: Option<&RangeInclusive<i32>>,
        frozen_origin: f64,
        mut visit: impl FnMut(&Self, i32, f64, f64),
    ) {
        let mut frozen_cursor = axis.strip_start();
        if let Some(band) = frozen_band {
            for i in band.clone() {
                let t = axis.extent(model, i);
                if t <= 0.0 {
                    continue;
                }
                visit(self, i, frozen_cursor, t);
                frozen_cursor += t;
            }
        }

        let mut scroll_cursor = if frozen_band.is_some() {
            frozen_origin
        } else {
            axis.strip_start()
        };
        for i in axis.visible_band(vis) {
            let t = axis.extent(model, i);
            if t <= 0.0 {
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

        self.set_fill_static(text_color);
        let center = full.center();
        let label = match axis {
            Axis::Row => index.to_string(),
            Axis::Column => col_name(index),
        };
        self.ctx
            .fill_text(&label, self.snap_pixel(center.x), self.snap_pixel(center.y))
            .ok();
    }
}
