//! Cell painting - pure pixel pusher.
//!
//! `render_pane` iterates the resolved-paint stream from `CellPaintsIter`
//! and hands each `CellPaint` to `paint_cell`. Nothing in this file talks
//! to the model: bg, borders, and text are pre-resolved upstream.

use crate::types::CellSlot;
use crate::{CanvasModel, Point, Span};

use super::super::geometry::{FrameContext, Line, PixelRect};
use super::super::model::{CellAddress, RCRange};
use super::super::types::{
    resolve_cell_paint, resolve_text_paint, BorderPaint, CellPaint, OuterEdge, PaneRegion,
};

use super::CanvasRenderer;

/// Which edge of a cell rectangle is being stroked.
///
/// `line()` projects the edge onto a `PixelRect` to produce the
/// axis-aligned `Line` segment painted by `paint_border`.
#[derive(Copy, Clone)]
pub(super) enum BorderEdge {
    Left,
    Top,
    Right,
    Bottom,
}

impl BorderEdge {
    /// The axis-aligned `Line` this edge would stroke on `rect`.
    fn line(self, rect: PixelRect) -> Line {
        let PixelRect {
            top_left: Point { x, y },
            width,
            height,
        } = rect;
        match self {
            BorderEdge::Left => Line::V {
                x,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Top => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y,
            },
            BorderEdge::Right => Line::V {
                x: x + width,
                span: Span {
                    from: y,
                    to: y + height,
                },
            },
            BorderEdge::Bottom => Line::H {
                span: Span {
                    from: x,
                    to: x + width,
                },
                y: y + height,
            },
        }
    }
}

/// Pane boundary edges to force-stroke around the active cell when it is
/// repainted on top of the selection overlay.
const ACTIVE_CELL_OUTER_EDGES: &[OuterEdge] = &[OuterEdge::Right, OuterEdge::Bottom];

impl CanvasRenderer {
    /// Walk one frozen-pane quadrant. Pass 1 paints bg+borders for every
    /// cell, then pass 2 paints text on top so overflow is never clipped by
    /// a neighbour's background.
    pub(super) fn render_pane(&self, model: &dyn CanvasModel, pane: PaneRegion) {
        let paints: Vec<CellPaint> = self.paints_in(model, &pane).collect();
        for p in &paints {
            self.paint_bg(p);
        }
        self.paint_borders_batched(&paints);
        self.paint_pane_text(model, &paints);
    }

    /// Fill a cell's background rectangle. Border pass is separate (batched).
    pub(super) fn paint_bg(&self, p: &CellPaint) {
        self.set_fill_cached(&p.bg);
        self.ctx_ref().fill_rect(
            p.rect.top_left.x,
            p.rect.top_left.y,
            p.rect.width,
            p.rect.height,
        );
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead; the main pane pass uses `paint_bg` + `paint_borders_batched`.
    pub(super) fn paint_cell(&self, p: &CellPaint) {
        self.paint_bg(p);
        self.paint_border(BorderEdge::Left, p.rect, &p.borders.left);
        self.paint_border(BorderEdge::Top, p.rect, &p.borders.top);
        if let Some(b) = &p.borders.right {
            self.paint_border(BorderEdge::Right, p.rect, b);
        }
        if let Some(b) = &p.borders.bottom {
            self.paint_border(BorderEdge::Bottom, p.rect, b);
        }
    }

    /// Pass 2: resolve and paint text for every cell in a collected pane.
    fn paint_pane_text(&self, model: &dyn CanvasModel, paints: &[CellPaint]) {
        // TEST: paints remove later
        // Damage tracking to skip cells
        // let mut c = 0;
        for p in paints {
            if let Some(t) = resolve_text_paint(self, model, p.addr, p.rect) {
                self.paint_text(&t);
                // c += 1;
            }
        }
        // web_sys::console::log_1(&format!("Paint painted texts: {}", c).into());
    }

    /// Batch all border lines in `paints` into per-style paths so each
    /// distinct (color, width) combination emits exactly one `begin_path` →
    /// N×`move_to`/`line_to` → `stroke()` sequence instead of one per edge.
    ///
    /// Linear Vec search is fastest here: a pane typically has 2–5 distinct
    /// border styles (grid gray + maybe 1–2 user-set colors).
    fn paint_borders_batched(&self, paints: &[CellPaint]) {
        struct Bucket {
            color: String,
            width_px: f64,
            lines: Vec<Line>,
        }

        let mut buckets: Vec<Bucket> = Vec::new();

        for p in paints {
            let edges: [Option<(BorderEdge, &BorderPaint)>; 4] = [
                Some((BorderEdge::Left, &p.borders.left)),
                Some((BorderEdge::Top, &p.borders.top)),
                p.borders.right.as_ref().map(|b| (BorderEdge::Right, b)),
                p.borders.bottom.as_ref().map(|b| (BorderEdge::Bottom, b)),
            ];
            for (edge, border) in edges.into_iter().flatten() {
                let base = edge.line(p.rect);
                // Double borders emit two offset lines into the same bucket.
                let double_lines = [base.offset_cross(-1.0), base.offset_cross(1.0)];
                let single_line = [base];
                let lines: &[Line] = if border.stroke.double {
                    &double_lines
                } else {
                    &single_line
                };
                for &line in lines {
                    if let Some(b) = buckets
                        .iter_mut()
                        .find(|b| b.color == border.color && b.width_px == border.stroke.width_px)
                    {
                        b.lines.push(line);
                    } else {
                        buckets.push(Bucket {
                            color: border.color.clone(),
                            width_px: border.stroke.width_px,
                            lines: vec![line],
                        });
                    }
                }
            }
        }

        for bucket in &buckets {
            self.set_stroke_cached(&bucket.color);
            self.set_line_width_cached(bucket.width_px);
            self.ctx_ref().begin_path();
            for line in &bucket.lines {
                match line {
                    Line::H { span, y } => {
                        self.ctx_ref().move_to(span.from, *y);
                        self.ctx_ref().line_to(span.to, *y);
                    }
                    Line::V { x, span } => {
                        self.ctx_ref().move_to(*x, span.from);
                        self.ctx_ref().line_to(*x, span.to);
                    }
                }
            }
            self.ctx_ref().stroke();
        }
    }

    /// Stroke one resolved border. `Double`-style borders render as two
    /// parallel strokes offset ±1px on the cross-axis.
    fn paint_border(&self, edge: BorderEdge, rect: PixelRect, b: &BorderPaint) {
        let line = edge.line(rect);
        let offsets: &[f64] = if b.stroke.double {
            &[-1.0, 1.0]
        } else {
            &[0.0]
        };
        self.set_stroke_cached(&b.color);
        self.set_line_width_cached(b.stroke.width_px);
        for &d in offsets {
            self.stroke_line(line.offset_cross(d));
        }
    }

    /// Repaint one cell's full paint (bg + borders + text).
    ///
    /// Used by the selection overlay to restore the active cell on top of
    /// the semi-transparent selection fill. Resolves the paint inline -
    /// neighbour styles are skipped (passed as `None`); the visual
    /// difference is imperceptible at a single cell boundary.
    pub(super) fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        addr: CellAddress,
        frame: &FrameContext,
    ) {
        let range = RCRange::from_cell(addr.row, addr.column);
        let Some(rect) = self.range_pixel_bounds(model, frame, range) else {
            return;
        };
        let Ok(own_style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };
        let show_grid = model.get_show_grid_lines(addr.sheet).unwrap_or(true);
        let Some(paint) = resolve_cell_paint(
            self,
            show_grid,
            CellSlot {
                addr,
                rect,
                outer_edges: ACTIVE_CELL_OUTER_EDGES,
            },
            &own_style,
            None,
            None,
        ) else {
            return;
        };
        self.paint_cell(&paint);
        if let Some(t) = resolve_text_paint(self, model, addr, rect) {
            self.paint_text(&t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_edge_is_vertical_line_at_rect_x() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Left.line(rect),
            Line::V {
                x: 5.0,
                span: Span {
                    from: 10.0,
                    to: 25.0,
                }
            }
        );
    }

    #[test]
    fn right_edge_is_vertical_line_at_rect_right() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Right.line(rect),
            Line::V {
                x: 25.0,
                span: Span {
                    from: 10.0,
                    to: 25.0,
                },
            }
        )
    }

    #[test]
    fn top_edge_is_horizontal_line_at_rect_y() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Top.line(rect),
            Line::H {
                y: 10.0,
                span: Span {
                    from: 5.0,
                    to: 25.0,
                },
            }
        );
    }

    #[test]
    fn bottom_edge_is_horizontal_line_at_rect_bottom() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            width: 20.0,
            height: 15.0,
        };
        assert_eq!(
            BorderEdge::Bottom.line(rect),
            Line::H {
                y: 25.0,
                span: Span {
                    from: 5.0,
                    to: 25.0,
                },
            }
        );
    }
}
