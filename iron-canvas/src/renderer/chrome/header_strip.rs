//! Row-number + column-letter header strips: base-pass painting and the
//! overlay-pass selection highlight.
//!
//! The base pass runs on the grid layer (`render_headers_base`) and paints
//! every visible header cell once per repaint. The highlight pass runs on
//! the overlay layer (`render_header_highlights`) and repaints just the
//! selected cells, so navigation events skip a grid repaint entirely.

use std::fmt::Write as _;

use crate::chrome::Chrome;
use crate::geometry::prim::Axis;
use crate::painter::{PaintColor, Painter, TextAlign, TextBaseline};
use crate::renderer::RendererCore;

const HEADER_FONT: &str = "bold 12px Inter, Arial, sans-serif";

impl<P: Painter> RendererCore<P> {
    /// Paint one header strip along `axis` with no selection highlighting.
    pub(crate) fn render_headers_base(&self, axis: Axis, frame: &Chrome) {
        self.walk_header_strip(axis, frame, |i, along, t| {
            self.draw_header_cell(axis, frame, i, along, t, false);
        });
    }

    /// Overlay pass repainting only selected header cells so the grid
    /// layer never redraws on navigation — the base pass beneath stays
    /// intact.
    pub(crate) fn render_header_highlights(&self, axis: Axis, frame: &Chrome) {
        let (sel_start, sel_end) = axis.selection_range(frame.selection_range);
        self.walk_header_strip(axis, frame, |i, along, t| {
            if i >= sel_start && i <= sel_end {
                self.draw_header_cell(axis, frame, i, along, t, true);
            }
        });
    }

    /// Walk the frozen band (if any) then the scrollable band, reading
    /// `(index, start, extent)` straight from the frame's slot vecs —
    /// the slots already carry absolute canvas coords, so no cursor
    /// accumulation is needed.
    fn walk_header_strip(&self, axis: Axis, frame: &Chrome, mut visit: impl FnMut(i32, i32, i32)) {
        match axis {
            Axis::Row => {
                for s in &frame.pane_set.frozen_rows {
                    visit(s.row, s.top, s.height);
                }
                for s in &frame.pane_set.scroll_rows {
                    visit(s.row, s.top, s.height);
                }
            }
            Axis::Column => {
                for s in &frame.pane_set.frozen_cols {
                    visit(s.col, s.left, s.width);
                }
                for s in &frame.pane_set.scroll_cols {
                    visit(s.col, s.left, s.width);
                }
            }
        }
    }

    /// Paint a single header cell: border strip, body fill, and label.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols); `thickness` is the cell's extent along the same axis (rh / cw).
    fn draw_header_cell(
        &self,
        axis: Axis,
        frame: &Chrome,
        index: i32,
        along: i32,
        thickness: i32,
        selected: bool,
    ) {
        let body_bg = PaintColor::from_theme_str(if selected {
            &frame.theme.header_selected_bg
        } else {
            &frame.theme.header_bg
        });
        let text_color = PaintColor::from_theme_str(if selected {
            &frame.theme.header_selected_color
        } else {
            &frame.theme.header_text_color
        });

        let header_thickness = match axis {
            Axis::Row => frame.row_header_thickness,
            Axis::Column => frame.col_header_thickness,
        };
        let full = axis.header_rect(along, thickness, header_thickness);
        // 1px inset on the cross-axis leaves the border strip visible
        // top+bottom (row) or left+right (column).
        let body = match axis {
            Axis::Row => full.inset(0, 1),
            Axis::Column => full.inset(1, 0),
        };

        self.painter.rect_fill(
            full,
            PaintColor::from_theme_str(&frame.theme.header_border_color),
        );
        self.painter.rect_fill(body, body_bg);
        let center = full.center();
        let snap_x = f64::from(center.x);
        let snap_y = f64::from(center.y);
        // Row labels: write the integer into the renderer-owned scratch
        // String (zero-alloc steady-state). Column labels: pull from the
        // per-renderer intern (one alloc per unique column over the
        // renderer's lifetime).
        match axis {
            Axis::Row => {
                let mut buf = self.frame_cache.label_buf.borrow_mut();
                buf.clear();
                let _ = write!(&mut *buf, "{}", index);
                self.painter.fill_text(
                    &buf,
                    snap_x,
                    snap_y,
                    PaintColor::Static(HEADER_FONT),
                    text_color,
                    TextAlign::Center,
                    TextBaseline::Middle,
                );
            }
            Axis::Column => {
                let label = self.col_intern.get(index);
                self.painter.fill_text(
                    &label,
                    snap_x,
                    snap_y,
                    PaintColor::Static(HEADER_FONT),
                    text_color,
                    TextAlign::Center,
                    TextBaseline::Middle,
                );
            }
        }
    }
}
