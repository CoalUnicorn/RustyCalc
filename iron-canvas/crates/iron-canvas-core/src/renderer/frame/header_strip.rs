//! Row-number + column-letter header strips: base-pass painting and the
//! overlay-pass selection highlight.
//!
//! The base pass runs on the grid layer (`render_headers_base`) and paints
//! every visible header cell once per repaint. The highlight pass runs on
//! the overlay layer (`render_header_highlights`) and repaints just the
//! selected cells, so navigation events skip a grid repaint entirely.

use crate::chrome::Chrome;
use crate::geometry::prim::Axis;
use crate::painter::{PaintColor, Painter, TextAlign, TextBaseline};
use crate::renderer::RendererCore;

const HEADER_FONT: &str = "bold 12px Inter, Arial, sans-serif";

impl<P: Painter> RendererCore<P> {
    /// Paint one header strip along `axis` with no selection highlighting.
    pub fn render_headers_base(&self, axis: Axis, frame: &Chrome) {
        self.walk_header_strip(axis, frame, |_i, along, t, label| {
            self.draw_header_cell(axis, frame, along, t, label, false);
        });
    }

    /// Overlay pass repainting only selected header cells so the grid
    /// layer never redraws on navigation — the base pass beneath stays
    /// intact.
    pub fn render_header_highlights(
        &self,
        axis: Axis,
        frame: &Chrome,
        selection_range: crate::types::coord::RCRange,
    ) {
        let (sel_start, sel_end) = axis.selection_range(selection_range);
        self.walk_header_strip(axis, frame, |i, along, t, label| {
            if i >= sel_start && i <= sel_end {
                self.draw_header_cell(axis, frame, along, t, label, true);
            }
        });
    }

    /// Walk the frozen band (if any) then the scrollable band, reading
    /// `(index, start, extent, label)` straight from the frame's slot vecs
    /// zipped against the parallel label vec — the slots already carry
    /// absolute canvas coords, so no cursor accumulation is needed.
    fn walk_header_strip(
        &self,
        axis: Axis,
        frame: &Chrome,
        mut visit: impl FnMut(i32, i32, i32, &str),
    ) {
        match axis {
            Axis::Row => {
                let labels = &frame.pane_set.row_header_labels;
                for (s, label) in frame
                    .pane_set
                    .frozen_rows
                    .iter()
                    .chain(frame.pane_set.scroll_rows.iter())
                    .zip(labels.iter())
                {
                    visit(s.row, s.top, s.height, label);
                }
            }
            Axis::Column => {
                let labels = &frame.pane_set.col_header_labels;
                for (s, label) in frame
                    .pane_set
                    .frozen_cols
                    .iter()
                    .chain(frame.pane_set.scroll_cols.iter())
                    .zip(labels.iter())
                {
                    visit(s.col, s.left, s.width, label);
                }
            }
        }
    }

    /// Paint a single header cell: border strip, body fill, and label.
    ///
    /// `along` is the position along the axis (top_y for rows, left_x for
    /// cols); `thickness` is the cell's extent along the same axis (rh / cw).
    /// `label` is the pre-resolved header text (model override or built-in),
    /// produced in `Chrome::build` where the model is in scope.
    fn draw_header_cell(
        &self,
        axis: Axis,
        frame: &Chrome,
        along: i32,
        thickness: i32,
        label: &str,
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
        self.painter.fill_text(
            label,
            snap_x,
            snap_y,
            PaintColor::Static(HEADER_FONT),
            text_color,
            TextAlign::Center,
            TextBaseline::Middle,
        );
    }
}
