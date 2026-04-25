//! Cell background + border rendering.
//!
//! `render_pane` walks the rows and columns of a single pane quadrant; for
//! each cell it calls `render_cell_style` to paint the fill and the four
//! border edges, then collects the text layout into a `Vec<CellText>` for
//! Phase 4.
//!
//! The four edges are resolved through a single `BorderEdge` enum + the two
//! `resolve_*_edge` helpers, each encoding its own fallback chain.

use ironcalc_base::types::{BorderItem, BorderStyle};

use crate::renderer::text::CellText;
use crate::{CanvasModel, Point, Span};

use super::super::geometry::{Line, PixelRect, SheetViewport};
use super::super::model::CellAddress;
use super::super::types::{OuterEdge, PaneRegion};

use super::{CanvasRenderer, MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

/// Which edge of a cell rectangle is being resolved.
///
/// `line()` turns it into the axis-aligned `Line` segment to stroke on a
/// given rect.
#[derive(Copy, Clone)]
enum BorderEdge {
    Left,
    Top,
    Right,
    Bottom,
}

/// Inner edges (left / top) — the only edges whose resolved color can be
/// borrowed from a neighbour cell's opposite border or fill.
/// Carved out as a separate enum from `BorderEdge` to avoid
/// "unreachable for Right/Bottom" arm.
#[derive(Copy, Clone)]
enum InnerEdge {
    Left,
    Top,
}

impl InnerEdge {
    /// Address of the neighbour whose opposite border/fill can influence this
    /// edge. Returns `None` at the grid's top-left boundary.
    fn neighbour(self, addr: CellAddress) -> Option<CellAddress> {
        match self {
            InnerEdge::Left if addr.column > 1 => Some(CellAddress {
                column: addr.column - 1,
                ..addr
            }),
            InnerEdge::Top if addr.row > 1 => Some(CellAddress {
                row: addr.row - 1,
                ..addr
            }),
            _ => None,
        }
    }

    fn as_edge(self) -> BorderEdge {
        match self {
            InnerEdge::Left => BorderEdge::Left,
            InnerEdge::Top => BorderEdge::Top,
        }
    }
}

impl BorderEdge {
    /// The axis-aligned `Line` this edge would draw on `rect`.
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

/// Resolve the left/top border — where a neighbour's opposite edge or fill
/// can influence the final color. Encodes the fallback chain:
/// own -> neighbour-side -> own-bg -> neighbour-bg -> grid.
fn resolve_inner_edge<'a>(
    own: Option<&'a BorderItem>,
    neighbour_side: Option<&'a BorderItem>,
    neighbour_bg: Option<&'a str>,
    own_bg_set: bool,
    bg: &'a str,
    grid_color: &'a str,
) -> (&'a str, &'a BorderStyle) {
    if let Some(b) = own {
        return (b.color.as_deref().unwrap_or(grid_color), &b.style);
    }
    if let Some(b) = neighbour_side {
        return (b.color.as_deref().unwrap_or(grid_color), &b.style);
    }
    if own_bg_set {
        return (bg, &BorderStyle::Thin);
    }
    if let Some(nbg) = neighbour_bg {
        return (nbg, &BorderStyle::Thin);
    }
    (grid_color, &BorderStyle::Thin)
}

/// Resolve the right/bottom border — simpler than inner edges: only drawn
/// when explicit or at a pane boundary, with no neighbour fallback.
fn resolve_outer_edge<'a>(
    own: Option<&'a BorderItem>,
    grid_color: &'a str,
) -> (&'a str, &'a BorderStyle) {
    if let Some(b) = own {
        (b.color.as_deref().unwrap_or(grid_color), &b.style)
    } else {
        (grid_color, &BorderStyle::Thin)
    }
}

impl CanvasRenderer {
    pub(super) fn render_pane(
        &self,
        model: &dyn CanvasModel,
        cell_texts: &mut Vec<CellText>,
        pane: PaneRegion,
    ) {
        let canvas = self.canvas_size();
        for cell in pane.cells(model, canvas) {
            self.render_cell_style(model, cell.addr, cell.rect, cell.outer_edges);
            if let Some(ct) = self.compute_cell_text(model, cell.addr, cell.rect) {
                cell_texts.push(ct);
            }
        }
    }

    /// Paint one cell's background and resolve/draw all four border edges.
    pub(super) fn render_cell_style(
        &self,
        model: &dyn CanvasModel,
        addr: CellAddress,
        rect: PixelRect,
        pane_outer: &[OuterEdge],
    ) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }

        let Ok(style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };

        let show_grid = model.get_show_grid_lines(addr.sheet).unwrap_or(true);
        let bg = style.fill.fg_color.as_deref().unwrap_or(self.theme.cell_bg);
        let grid_color = if show_grid { self.theme.grid_color } else { bg };
        let own_bg_set = style.fill.fg_color.is_some();

        // Background fill.
        self.ctx.set_fill_style_str(bg);
        self.ctx
            .fill_rect(rect.top_left.x, rect.top_left.y, rect.width, rect.height);

        // Inner edges (left, top) — each falls back to the matching neighbour's
        // opposite border and fill.
        for inner in [InnerEdge::Left, InnerEdge::Top] {
            let own = match inner {
                InnerEdge::Left => style.border.left.as_ref(),
                InnerEdge::Top => style.border.top.as_ref(),
            };

            let neighbour_style = if own.is_none() && !own_bg_set {
                inner
                    .neighbour(addr)
                    .and_then(|a| model.get_cell_style(a.sheet, a.row, a.column).ok())
            } else {
                None
            };

            let neighbour_border = match inner {
                InnerEdge::Left => neighbour_style
                    .as_ref()
                    .and_then(|n| n.border.right.as_ref()),
                InnerEdge::Top => neighbour_style
                    .as_ref()
                    .and_then(|n| n.border.bottom.as_ref()),
            };

            let (color, border_style) = resolve_inner_edge(
                own,
                neighbour_border,
                neighbour_style
                    .as_ref()
                    .and_then(|n| n.fill.fg_color.as_deref()),
                own_bg_set,
                bg,
                grid_color,
            );
            self.draw_border(inner.as_edge(), rect, border_style, color);
        }

        // Right edge — only at pane boundary or when explicitly set.
        if pane_outer.contains(&OuterEdge::Right) || style.border.right.is_some() {
            let (rc, rs) = resolve_outer_edge(style.border.right.as_ref(), grid_color);
            self.draw_border(BorderEdge::Right, rect, rs, rc);
        }

        // Bottom edge — only at pane boundary or when explicitly set.
        if pane_outer.contains(&OuterEdge::Bottom) || style.border.bottom.is_some() {
            let (bc, bs) = resolve_outer_edge(style.border.bottom.as_ref(), grid_color);
            self.draw_border(BorderEdge::Bottom, rect, bs, bc);
        }
    }

    /// Repaint one cell's background + borders.
    ///
    /// Used by selection overlay to restore the active cell's real style on
    /// top of the semi-transparent selection fill. Computes pixel position
    /// via `SheetViewport` so the caller only needs logical `(row,column)`.
    pub(super) fn repaint_active_cell(&self, model: &dyn CanvasModel, addr: CellAddress) {
        let rect = SheetViewport::current(model).cell_rect(addr.row, addr.column);
        self.render_cell_style(model, addr, rect, &[OuterEdge::Right, OuterEdge::Bottom]);
    }

    fn draw_border(&self, edge: BorderEdge, rect: PixelRect, style: &BorderStyle, color: &str) {
        let line = edge.line(rect);
        let width = match style {
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot => MEDIUM_BORDER_WIDTH,
            BorderStyle::Thick => THICK_BORDER_WIDTH,
            // Thin, Dotted, Double, SlantDashDot, etc. -> one pixel wide.
            _ => STANDARD_BORDER_WIDTH,
        };
        // Double renders as two parallel thin lines offset ±1px on the cross-axis;
        // every other style is a single line on the segment itself.
        let offsets: &[f64] = if matches!(style, BorderStyle::Double) {
            &[-1.0, 1.0]
        } else {
            &[0.0]
        };

        self.ctx.save();
        self.ctx.set_stroke_style_str(color);
        self.with_stroke_width(width, |this| {
            for &d in offsets {
                this.stroke_line(line.offset_cross(d));
            }
        });
        self.ctx.restore();
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

    #[test]
    fn left_neighbour_decrements_column() {
        let addr = CellAddress {
            sheet: 0,
            row: 3,
            column: 4,
        };
        assert_eq!(
            InnerEdge::Left.neighbour(addr),
            Some(CellAddress {
                sheet: 0,
                row: 3,
                column: 3,
            })
        );
    }

    #[test]
    fn top_neighbour_decrements_row() {
        let addr = CellAddress {
            sheet: 0,
            row: 3,
            column: 4,
        };
        assert_eq!(
            InnerEdge::Top.neighbour(addr),
            Some(CellAddress {
                sheet: 0,
                row: 2,
                column: 4,
            })
        );
    }

    #[test]
    fn left_neighbour_at_column_one_is_none() {
        let addr = CellAddress {
            sheet: 0,
            row: 10,
            column: 1,
        };
        assert_eq!(InnerEdge::Left.neighbour(addr), None);
    }

    #[test]
    fn top_neighbour_at_row_one_is_none() {
        let addr = CellAddress {
            sheet: 0,
            row: 1,
            column: 10,
        };
        assert_eq!(InnerEdge::Top.neighbour(addr), None);
    }
}
