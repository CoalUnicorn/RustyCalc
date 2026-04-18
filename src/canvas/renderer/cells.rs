//! Cell background + border rendering.
//!
//! `render_pane` walks the rows andcolumnumns of a single pane quadrant; for
//! each cell it calls `render_cell_style` to paint the fill and the four
//! border edges, thencolumnlects the text layout into a `Vec<CellText>` for
//! Phase 4.
//!
//! The four edges are resolved through a single `BorderEdge` enum + the two
//! `resolve_*_edge` helpers — what used to be four near-identical L/T/R/B
//! branches.

use ironcalc_base::types::{BorderItem, BorderStyle};
use ironcalc_base::UserModel;

use crate::canvas::Point;
use crate::coord::CellAddress;

use super::super::geometry::{col_width, row_height, Line, PixelRect};
use super::super::types::{CellEdges, CellText, FrozenOffset, PaneRegion};
use super::{CanvasRenderer, MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

//  Local border primitives

/// Which edge of a cell rectangle is being resolved.
///
/// loop per cell that asks the enum for its `Line` on a given rect.
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
            point: Point { x, y },
            width,
            height,
        } = rect;
        match self {
            BorderEdge::Left => Line::V {
                x,
                y1: y,
                y2: y + height,
            },
            BorderEdge::Top => Line::H {
                x1: x,
                x2: x + width,
                y,
            },
            BorderEdge::Right => Line::V {
                x: x + width,
                y1: y,
                y2: y + height,
            },
            BorderEdge::Bottom => Line::H {
                x1: x,
                x2: x + width,
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
        model: &UserModel,
        sheet: u32,
        cell_texts: &mut Vec<CellText>,
        pane: PaneRegion,
    ) {
        if pane.rows.is_empty() || pane.cols.is_empty() {
            return;
        }

        let column_range = pane.cols.clone();
        let column_count = (column_range.end() - column_range.start() + 1) as usize;
        let mut column_widths = Vec::with_capacity(column_count);
        for column in column_range {
            column_widths.push((column, col_width(model, sheet, column)));
        }

        let mut y = pane.start_y;
        for row in pane.rows {
            if y >= self.height {
                break;
            }
            let rh = row_height(model, sheet, row);
            if rh <= 0.0 {
                continue;
            }

            let mut x = pane.start_x;
            for (column, cw) in &column_widths {
                if x >= self.width {
                    break;
                }
                if *cw <= 0.0 {
                    x += cw;
                    continue;
                }

                let rect = PixelRect {
                    point: Point { x, y },
                    width: *cw,
                    height: rh,
                };
                let addr = CellAddress {
                    sheet,
                    row,
                    column: *column,
                };

                if self.is_rect_visible(rect) {
                    self.render_cell_style(
                        model,
                        addr,
                        rect,
                        CellEdges {
                            right: *column == pane.last_col,
                            bottom: row == pane.last_row,
                        },
                    );

                    if let Some(ct) = self.compute_cell_text(model, addr, rect) {
                        cell_texts.push(ct);
                    }
                }

                x += cw;
            }
            y += rh;
        }
    }

    /// Check if a rectangle is at least partially visible on the canvas.
    pub(super) fn is_rect_visible(&self, rect: PixelRect) -> bool {
        rect.point.x < self.width
            && (rect.point.x + rect.width) > 0.0
            && rect.point.y < self.height
            && (rect.point.y + rect.height) > 0.0
    }

    /// Paint one cell's background and resolve/draw all four border edges.
    pub(super) fn render_cell_style(
        &self,
        model: &UserModel,
        addr: CellAddress,
        rect: PixelRect,
        edges: CellEdges,
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
            .fill_rect(rect.point.x, rect.point.y, rect.width, rect.height);

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
        if edges.right || style.border.right.is_some() {
            let (rc, rs) = resolve_outer_edge(style.border.right.as_ref(), grid_color);
            self.draw_border(BorderEdge::Right, rect, rs, rc);
        }

        // Bottom edge — only at pane boundary or when explicitly set.
        if edges.bottom || style.border.bottom.is_some() {
            let (bc, bs) = resolve_outer_edge(style.border.bottom.as_ref(), grid_color);
            self.draw_border(BorderEdge::Bottom, rect, bs, bc);
        }
    }

    /// Repaint one cell's background + borders.
    ///
    /// Used by selection overlay to restore the active cell's real style on
    /// top of the semi-transparent selection fill. Computes pixel position
    /// internally so the caller only needs logical `(row,column)`.
    pub(super) fn repaint_active_cell(
        &self,
        model: &UserModel,
        addr: CellAddress,
        frozen: FrozenOffset,
    ) {
        let rect = PixelRect {
            point: Point {
                x: self.cell_x(model, addr.sheet, addr.column, frozen),
                y: self.cell_y(model, addr.sheet, addr.row, frozen),
            },
            width: col_width(model, addr.sheet, addr.column),
            height: row_height(model, addr.sheet, addr.row),
        };
        self.render_cell_style(
            model,
            addr,
            rect,
            CellEdges {
                right: true,
                bottom: true,
            },
        );
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
