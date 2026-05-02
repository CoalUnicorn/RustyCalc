//! Pane-wide gridlines — one stroke per visible row/column, never per cell.
//!
//! Cells own only their fill (and explicit `BorderItem` strokes); the soft
//! "grid" between cells is painted here as full-extent lines on each frozen
//! pane. The cost model is one canvas stroke per row-top + one per column-left,
//! independent of the cell count, so a 100×40 pane is ~140 strokes instead of
//! 4·4·100·40 = 64 000 per-edge strokes.
//!
//! ## Geometry
//!
//! Every pane is a rectangle whose interior is fully described by two
//! prefix-sum vectors that `FrameContext` already builds for cell layout:
//!
//! * `rows`: cumulative Y-offsets within the pane, length `n_rows + 1`.
//!   Entry `[0]` is the pane top, `[last]` the pane bottom.
//! * `cols`: same shape on the X-axis.
//!
//! Interior gridlines are exactly the entries `[1..last]` — the endpoints
//! coincide with the header strip or the frozen separator, both of which are
//! drawn by their own passes (`render_headers_base`, `draw_frozen_separators`),
//! so we leave those entries to them.
//!
//! ## Pipeline shape
//!
//! `GridLinesPaint::resolve` snapshots the lines once per repaint, then
//! `CanvasRenderer::paint_grid_lines` ships them with a single stroke-style
//! and line-width set, plus a tight loop over `stroke_line`. No model access,
//! no per-cell allocation. Wiring is intentionally absent — `GridLayer` will
//! call `paint_grid_lines` once it adopts this module.

use crate::geometry::{
    FrameContext, Line, Point, Span, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT,
};
use crate::renderer::STANDARD_BORDER_WIDTH;
use crate::theme::CanvasTheme;
use crate::CanvasRenderer;

/// Resolved gridline set for the entire painted frame.
///
/// `strokes` mixes horizontal and vertical lines in pane order; the painter
/// dispatches per-line via `Line`'s enum variant. `color` and `width_px` are
/// shared across every stroke, so the painter sets them exactly once.
pub(crate) struct GridLinesPaint {
    pub strokes: Vec<Line>,
    pub color: &'static str,
    pub width_px: f64,
}

impl GridLinesPaint {
    /// Build the line set for `frame` once per repaint.
    ///
    /// Walks each frozen-pane quadrant's prefix-sum offsets and emits one
    /// horizontal line per interior row-top and one vertical line per
    /// interior column-left. Capacity is exact (sum of `line_count` over
    /// panes), so the backing vector never reallocates during the walk.
    pub(crate) fn resolve(theme: &CanvasTheme, frame: &FrameContext) -> Self {
        let panes = PaneBox::all(frame);
        let cap: usize = panes.iter().map(PaneBox::line_count).sum();
        let mut strokes = Vec::with_capacity(cap);
        for pane in &panes {
            pane.append_lines(&mut strokes);
        }
        Self {
            strokes,
            color: theme.grid_color,
            width_px: STANDARD_BORDER_WIDTH,
        }
    }
}

/// One frozen-pane quadrant projected as prefix sums + an absolute origin.
///
/// `rows` and `cols` are borrowed straight from `PixelOffsets`; the origin
/// is the pane's top-left in canvas pixels (header strip for frozen panes,
/// `FrozenRC::offset` for scrollable ones). All lines are derived by
/// translating prefix-sum entries through `origin`.
struct PaneBox<'a> {
    origin: Point,
    rows: &'a [f64],
    cols: &'a [f64],
}

impl<'a> PaneBox<'a> {
    /// Build the up-to-four pane boxes that cover the painted region. Frozen
    /// quadrants are skipped when their band is empty, mirroring how the
    /// cell renderer skips frozen `render_pane` calls in `render_grid`.
    fn all(frame: &'a FrameContext) -> Vec<PaneBox<'a>> {
        let off = &frame.offsets;
        let frz = &frame.frozen;
        let header = Point {
            x: HEADER_COL_WIDTH,
            y: HEADER_ROW_HEIGHT,
        };
        let scroll = frz.offset;
        let has_rows = frz.row_band.is_some();
        let has_cols = frz.col_band.is_some();

        let mut panes = Vec::with_capacity(4);
        if has_rows && has_cols {
            panes.push(PaneBox {
                origin: header,
                rows: &off.frozen_row_tops,
                cols: &off.frozen_col_lefts,
            });
        }
        if has_rows {
            panes.push(PaneBox {
                origin: Point {
                    x: scroll.x,
                    y: header.y,
                },
                rows: &off.frozen_row_tops,
                cols: &off.col_lefts,
            });
        }
        if has_cols {
            panes.push(PaneBox {
                origin: Point {
                    x: header.x,
                    y: scroll.y,
                },
                rows: &off.row_tops,
                cols: &off.frozen_col_lefts,
            });
        }
        panes.push(PaneBox {
            origin: scroll,
            rows: &off.row_tops,
            cols: &off.col_lefts,
        });
        panes
    }

    /// Exact number of lines this pane contributes — `(n_rows-1) + (n_cols-1)`,
    /// floored at zero. Lets `resolve` size the output vector with no slack.
    fn line_count(&self) -> usize {
        self.rows.len().saturating_sub(2) + self.cols.len().saturating_sub(2)
    }

    /// Push interior gridlines into `out`. Endpoints are skipped — those are
    /// the pane's outer edges, owned by `render_headers_base` and
    /// `draw_frozen_separators`.
    fn append_lines(&self, out: &mut Vec<Line>) {
        let nr = self.rows.len();
        let nc = self.cols.len();
        if nr < 2 || nc < 2 {
            return;
        }

        let x_from = self.origin.x + self.cols[0];
        let x_to = self.origin.x + self.cols[nc - 1];
        let y_from = self.origin.y + self.rows[0];
        let y_to = self.origin.y + self.rows[nr - 1];

        for &dy in &self.rows[1..nr - 1] {
            out.push(Line::H {
                y: self.origin.y + dy,
                span: Span {
                    from: x_from,
                    to: x_to,
                },
            });
        }
        for &dx in &self.cols[1..nc - 1] {
            out.push(Line::V {
                x: self.origin.x + dx,
                span: Span {
                    from: y_from,
                    to: y_to,
                },
            });
        }
    }
}

impl CanvasRenderer {
    /// Stroke every line in `paint` with a single style/width set.
    ///
    /// `with_stroke_width` restores the standard width on exit so cell
    /// border passes downstream don't see a stale `line_width`. Stroke
    /// snapping happens inside `stroke_line` (per `snap_stroke`), so 1-px
    /// gridlines still land on a single device pixel.
    pub(super) fn paint_grid_lines(&self, paint: &GridLinesPaint) {
        if paint.strokes.is_empty() {
            return;
        }
        self.set_stroke_static(paint.color);
        self.with_stroke_width(paint.width_px, |r| {
            for &line in &paint.strokes {
                r.stroke_line(line);
            }
        });
    }
}
