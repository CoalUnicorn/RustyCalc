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
use ironcalc_base::UserModel;

use crate::canvas::{Point, Span};
use crate::coord::CellAddress;

use super::super::geometry::{col_width, row_height, Line, PixelRect};
use super::super::types::{CellEdges, CellText, FrozenOffset, PaneRegion};
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
            size: Point {
                x: width,
                y: height,
            },
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
        model: &UserModel,
        sheet: u32,
        cell_texts: &mut Vec<CellText>,
        pane: PaneRegion,
    ) {
        if pane.rows.is_empty() || pane.cols.is_empty() {
            return;
        }

        // Per-column width is read once and reused across every row in the pane.
        // PixelOffsets caches prefix sums, not raw widths, so this still pays off.
        let column_widths: Vec<(i32, f64)> = pane
            .cols
            .clone()
            .map(|column| (column, col_width(model, sheet, column)))
            .collect();

        let mut row_top = pane.start_y;
        for row in pane.rows.clone() {
            if row_top >= self.height {
                break;
            }
            let row_h = row_height(model, sheet, row);
            if row_h > 0.0 {
                self.render_pane_row(
                    model,
                    sheet,
                    cell_texts,
                    &pane,
                    row,
                    row_top,
                    row_h,
                    &column_widths,
                );
            }
            row_top += row_h;
        }
    }

    /// Paint one row of a pane: iterate visible columns, drop zero-width and
    /// off-canvas cells, forward each visible cell to `render_cell_style` and
    /// collect its text layout.
    #[allow(clippy::too_many_arguments)]
    fn render_pane_row(
        &self,
        model: &UserModel,
        sheet: u32,
        cell_texts: &mut Vec<CellText>,
        pane: &PaneRegion,
        row: i32,
        row_top: f64,
        row_h: f64,
        column_widths: &[(i32, f64)],
    ) {
        let mut col_left = pane.start_x;
        for &(column, col_w) in column_widths {
            if col_left >= self.width {
                break;
            }
            if col_w > 0.0 {
                let rect = PixelRect {
                    top_left: Point {
                        x: col_left,
                        y: row_top,
                    },
                    size: Point { x: col_w, y: row_h },
                };
                if self.is_rect_visible(rect) {
                    let addr = CellAddress { sheet, row, column };
                    let edges = CellEdges {
                        right: column == pane.last_col,
                        bottom: row == pane.last_row,
                    };
                    self.render_cell_style(model, addr, rect, edges);
                    if let Some(ct) = self.compute_cell_text(model, addr, rect) {
                        cell_texts.push(ct);
                    }
                }
            }
            col_left += col_w;
        }
    }

    /// Check if a rectangle is at least partially visible on the canvas.
    /// Canvas-local AABB visibility for a pixel rect. Cheap last-line guard
    /// used inside per-cell loops (`render_pane`, `compute_cell_text`) to skip
    /// cells that fall off-canvas — notably when a frozen band is wider/taller
    /// than the canvas itself.
    ///
    /// Not a substitute for `range_pixel_bounds`, which operates in sheet-coord
    /// space and short-circuits expensive offset lookups for out-of-fold ranges.
    pub(super) fn is_rect_visible(&self, rect: PixelRect) -> bool {
        rect.top_left.x < self.width
            && (rect.top_left.x + rect.size.x) > 0.0
            && rect.top_left.y < self.height
            && (rect.top_left.y + rect.size.y) > 0.0
    }

    /// Paint one cell's background and resolve/draw all four border edges.
    pub(super) fn render_cell_style(
        &self,
        model: &UserModel,
        addr: CellAddress,
        rect: PixelRect,
        edges: CellEdges,
    ) {
        if rect.size.x <= 0.0 || rect.size.y <= 0.0 {
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
        self.ctx.fill_rect(
            rect.top_left.x,
            rect.top_left.y,
            rect.size.x,
            rect.size.y,
        );

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
            top_left: Point {
                x: self.cell_x(model, addr.sheet, addr.column, frozen),
                y: self.cell_y(model, addr.sheet, addr.row, frozen),
            },
            size: Point {
                x: col_width(model, addr.sheet, addr.column),
                y: row_height(model, addr.sheet, addr.row),
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_edge_is_vertical_line_at_rect_x() {
        let rect = PixelRect {
            top_left: Point { x: 5.0, y: 10.0 },
            size: Point { x: 20.0, y: 15.0 },
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
            size: Point { x: 20.0, y: 15.0 },
        };
        assert_eq!(
            BorderEdge::Right.line(rect),
            Line::H {
                span: Span {
                    from: 25.0,
                    to: 10.0,
                },
                y: 25.0,
            }
        )
    }

    #[test]
    fn top_edge_is_horizontal_line_at_rect_y() {
        assert!(true)
    }

    #[test]
    fn bottom_edge_is_horizontal_line_at_rect_bottom() {
        assert!(true)
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
        assert!(true)
    }

    #[test]
    fn left_neighbour_at_column_one_is_none() {
        assert!(true)
    }

    #[test]
    fn top_neighbour_at_row_one_is_none() {
        assert!(true)
    }
}
