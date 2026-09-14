//! Row-number + column-letter header strips: base-pass painting and the
//! overlay-pass selection highlight.
//!
//! The base pass runs on the grid layer (`render_headers_base`) and paints
//! every visible header cell once per repaint. The highlight pass runs on
//! the overlay layer (`render_header_highlights`) and repaints just the
//! selected cells, so navigation events skip a grid repaint entirely.

use crate::chrome::Chrome;
use crate::geometry::prim::Axis;
use crate::geometry::slot::{ColSlot, RowSlot};
use crate::painter::{PaintColor, Painter, TextAlign, TextBaseline};
use crate::renderer::RendererCore;

const HEADER_FONT: &str = "bold 12px Inter, Arial, sans-serif";

/// One header cell during the strip walk.
///
/// The variant names the axis, so the walk carries the named slot type
/// instead of the decomposed `(index, start, extent)` triple: those three
/// scalars mean `(row, top, height)` on one axis and `(col, left, width)` on
/// the other, so a transposed pair at a call site would compile. `axis()`
/// gives `draw_header_cell` the cross-axis thickness from the same value that
/// carries the along-axis geometry.
#[derive(Clone, Copy, Debug)]
enum HeaderSlot {
    Row(RowSlot),
    Col(ColSlot),
}

impl HeaderSlot {
    fn axis(self) -> Axis {
        match self {
            Self::Row(_) => Axis::Row,
            Self::Col(_) => Axis::Column,
        }
    }

    /// Cell index along the strip (`row` / `col`).
    fn index(self) -> i32 {
        match self {
            Self::Row(slot) => slot.row,
            Self::Col(slot) => slot.col,
        }
    }

    /// Leading edge along the strip axis (top y / left x).
    fn start(self) -> i32 {
        match self {
            Self::Row(slot) => slot.top,
            Self::Col(slot) => slot.left,
        }
    }

    /// Extent along the strip axis (height / width).
    fn extent(self) -> i32 {
        match self {
            Self::Row(slot) => slot.height,
            Self::Col(slot) => slot.width,
        }
    }
}

impl<P: Painter> RendererCore<P> {
    /// Paint one header strip along `axis` with no selection highlighting.
    pub fn render_headers_base(&self, axis: Axis, frame: &Chrome) {
        self.walk_header_strip(axis, frame, |slot, label| {
            self.draw_header_cell(frame, slot, label, false);
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
        self.walk_header_strip(axis, frame, |slot, label| {
            let index = slot.index();
            if index >= sel_start && index <= sel_end {
                self.draw_header_cell(frame, slot, label, true);
            }
        });
    }

    /// Walk the frozen band (if any) then the scrollable band, reading each
    /// slot straight from the frame's slot vecs zipped against the parallel
    /// label vec — the slots already carry absolute canvas coords, so no
    /// cursor accumulation is needed.
    fn walk_header_strip(
        &self,
        axis: Axis,
        frame: &Chrome,
        mut visit: impl FnMut(HeaderSlot, &str),
    ) {
        match axis {
            Axis::Row => {
                let labels = &frame.pane_set.row_header_labels;
                for (slot, label) in frame
                    .pane_set
                    .rows
                    .frozen
                    .iter()
                    .chain(frame.pane_set.rows.scroll.iter())
                    .zip(labels.iter())
                {
                    visit(HeaderSlot::Row(*slot), label);
                }
            }
            Axis::Column => {
                let labels = &frame.pane_set.col_header_labels;
                for (slot, label) in frame
                    .pane_set
                    .cols
                    .frozen
                    .iter()
                    .chain(frame.pane_set.cols.scroll.iter())
                    .zip(labels.iter())
                {
                    visit(HeaderSlot::Col(*slot), label);
                }
            }
        }
    }

    /// Paint a single header cell: border strip, body fill, and label.
    ///
    /// `label` is the pre-resolved header text (model override or built-in),
    /// produced in `Chrome::build` where the model is in scope.
    fn draw_header_cell(&self, frame: &Chrome, slot: HeaderSlot, label: &str, selected: bool) {
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

        let axis = slot.axis();
        let header_thickness = match axis {
            Axis::Row => frame.row_header_thickness,
            Axis::Column => frame.col_header_thickness,
        };
        let full = axis.header_rect(slot.start(), slot.extent(), header_thickness);
        // 1px inset along the strip axis leaves the border strip visible
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
