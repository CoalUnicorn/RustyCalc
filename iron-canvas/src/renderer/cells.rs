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
            self.paint_cell(p);
        }
        self.paint_pane_text(model, &paints);
    }

    /// Paint bg + borders for one resolved `CellPaint`. Text is handled
    /// separately in `paint_pane_text`.
    pub(super) fn paint_cell(&self, p: &CellPaint) {
        self.ctx_ref().set_fill_style_str(&p.bg);
        self.ctx_ref().fill_rect(
            p.rect.top_left.x,
            p.rect.top_left.y,
            p.rect.width,
            p.rect.height,
        );

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
        for p in paints {
            if let Some(t) = resolve_text_paint(self, model, p.addr, p.rect) {
                self.paint_text(&t);
            }
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

        self.ctx_ref().save();
        self.ctx_ref().set_stroke_style_str(&b.color);
        self.with_stroke_width(b.stroke.width_px, |this| {
            for &d in offsets {
                this.stroke_line(line.offset_cross(d));
            }
        });
        self.ctx_ref().restore();
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
