//! Cell background + border rendering.
//!
//! `render_pane` walks the rows and columns of a single pane quadrant; for
//! each cell it calls `render_cell_style` to paint the fill and the four
//! border edges, then collects the text layout into a `Vec<CellText>` for
//! Phase 4.
//!
//! The four edges are resolved through a single `BorderEdge` enum + the two
//! `resolve_*_edge` helpers — what used to be four near-identical L/T/R/B
//! branches.

use ironcalc_base::types::{BorderItem, BorderStyle};
use ironcalc_base::UserModel;
use web_sys::CanvasRenderingContext2d;

use super::super::geometry::{col_width, row_height, PixelRect};
use super::super::types::{CellEdges, CellText, FrozenOffset, PaneRegion};
use super::{CanvasRenderer, MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

//  Local border primitives

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BorderOrientation {
    Vertical,
    Horizontal,
}

/// Line segment passed to the border-drawing helper.
///
/// A two-point line (`x1,y1` → `x2,y2`), distinct from `PixelRect`.
pub(super) struct BorderSegment {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
}

/// Which edge of a cell rectangle is being resolved.
///
/// Lets the four previously-duplicated L/T/R/B branches collapse into one
/// loop per cell that asks the enum for orientation + segment.
#[derive(Copy, Clone)]
enum BorderEdge {
    Left,
    Top,
    Right,
    Bottom,
}

impl BorderEdge {
    fn orientation(self) -> BorderOrientation {
        match self {
            BorderEdge::Left | BorderEdge::Right => BorderOrientation::Vertical,
            BorderEdge::Top | BorderEdge::Bottom => BorderOrientation::Horizontal,
        }
    }

    fn segment(self, rect: PixelRect) -> BorderSegment {
        let PixelRect { x, y, width, height } = rect;
        match self {
            BorderEdge::Left => BorderSegment {
                x1: x,
                y1: y,
                x2: x,
                y2: y + height,
            },
            BorderEdge::Top => BorderSegment {
                x1: x,
                y1: y,
                x2: x + width,
                y2: y,
            },
            BorderEdge::Right => BorderSegment {
                x1: x + width,
                y1: y,
                x2: x + width,
                y2: y + height,
            },
            BorderEdge::Bottom => BorderSegment {
                x1: x,
                y1: y + height,
                x2: x + width,
                y2: y + height,
            },
        }
    }
}

/// Resolve the left/top border — where a neighbour's opposite edge or fill
/// can influence the final colour. Encodes the fallback chain:
/// own → neighbour-side → own-bg → neighbour-bg → grid.
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

        let col_range = pane.cols.clone();
        let col_count = (col_range.end() - col_range.start() + 1) as usize;
        let mut col_widths = Vec::with_capacity(col_count);
        for col in col_range {
            col_widths.push((col, col_width(model, sheet, col)));
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
            for (col, cw) in &col_widths {
                if x >= self.width {
                    break;
                }
                if *cw <= 0.0 {
                    x += cw;
                    continue;
                }

                let rect = PixelRect {
                    x,
                    y,
                    width: *cw,
                    height: rh,
                };

                if self.is_rect_visible(rect) {
                    self.render_cell_style(
                        model,
                        sheet,
                        row,
                        *col,
                        rect,
                        CellEdges {
                            right: *col == pane.last_col,
                            bottom: row == pane.last_row,
                        },
                    );

                    if let Some(ct) = self.compute_cell_text(model, sheet, row, *col, rect) {
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
        rect.x < self.width
            && (rect.x + rect.width) > 0.0
            && rect.y < self.height
            && (rect.y + rect.height) > 0.0
    }

    /// Paint one cell's background and resolve/draw all four border edges.
    pub(super) fn render_cell_style(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        col: i32,
        rect: PixelRect,
        edges: CellEdges,
    ) {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }

        let Ok(style) = model.get_cell_style(sheet, row, col) else {
            return;
        };

        let show_grid = model.get_show_grid_lines(sheet).unwrap_or(true);
        let bg = style.fill.fg_color.as_deref().unwrap_or(self.theme.cell_bg);
        let grid_color = if show_grid { self.theme.grid_color } else { bg };
        let own_bg_set = style.fill.fg_color.is_some();

        // Background fill.
        self.ctx.set_fill_style_str(bg);
        self.ctx.fill_rect(rect.x, rect.y, rect.width, rect.height);

        // Left edge — fall back to left neighbour's right border and fill.
        let left_nb = if col > 1 && style.border.left.is_none() && !own_bg_set {
            model.get_cell_style(sheet, row, col - 1).ok()
        } else {
            None
        };
        let (lc, ls) = resolve_inner_edge(
            style.border.left.as_ref(),
            left_nb.as_ref().and_then(|n| n.border.right.as_ref()),
            left_nb.as_ref().and_then(|n| n.fill.fg_color.as_deref()),
            own_bg_set,
            bg,
            grid_color,
        );
        self.draw_edge(BorderEdge::Left, rect, ls, lc);

        // Top edge — fall back to top neighbour's bottom border and fill.
        let top_nb = if row > 1 && style.border.top.is_none() && !own_bg_set {
            model.get_cell_style(sheet, row - 1, col).ok()
        } else {
            None
        };
        let (tc, ts) = resolve_inner_edge(
            style.border.top.as_ref(),
            top_nb.as_ref().and_then(|n| n.border.bottom.as_ref()),
            top_nb.as_ref().and_then(|n| n.fill.fg_color.as_deref()),
            own_bg_set,
            bg,
            grid_color,
        );
        self.draw_edge(BorderEdge::Top, rect, ts, tc);

        // Right edge — only at pane boundary or when explicitly set.
        if edges.right || style.border.right.is_some() {
            let (rc, rs) = resolve_outer_edge(style.border.right.as_ref(), grid_color);
            self.draw_edge(BorderEdge::Right, rect, rs, rc);
        }

        // Bottom edge — only at pane boundary or when explicitly set.
        if edges.bottom || style.border.bottom.is_some() {
            let (bc, bs) = resolve_outer_edge(style.border.bottom.as_ref(), grid_color);
            self.draw_edge(BorderEdge::Bottom, rect, bs, bc);
        }
    }

    /// Repaint one cell's background + borders.
    ///
    /// Used by selection overlay to restore the active cell's real style on
    /// top of the semi-transparent selection fill. Computes pixel position
    /// internally so the caller only needs logical `(row, col)`.
    pub(super) fn repaint_active_cell(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        col: i32,
        frozen: FrozenOffset,
    ) {
        let rect = PixelRect {
            x: self.cell_x(model, sheet, col, frozen),
            y: self.cell_y(model, sheet, row, frozen),
            width: col_width(model, sheet, col),
            height: row_height(model, sheet, row),
        };
        self.render_cell_style(
            model,
            sheet,
            row,
            col,
            rect,
            CellEdges {
                right: true,
                bottom: true,
            },
        );
    }

    fn draw_edge(
        &self,
        edge: BorderEdge,
        rect: PixelRect,
        style: &BorderStyle,
        color: &str,
    ) {
        let seg = edge.segment(rect);
        self.draw_border(&seg, style, color, edge.orientation());
    }

    pub(super) fn draw_border(
        &self,
        seg: &BorderSegment,
        style: &BorderStyle,
        color: &str,
        orientation: BorderOrientation,
    ) {
        let BorderSegment { x1, y1, x2, y2 } = *seg;
        let ctx = &self.ctx;
        ctx.save();
        ctx.set_stroke_style_str(color);
        match style {
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot => {
                ctx.set_line_width(MEDIUM_BORDER_WIDTH);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
            BorderStyle::Thick => {
                ctx.set_line_width(THICK_BORDER_WIDTH);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
            BorderStyle::Double => {
                ctx.set_line_width(STANDARD_BORDER_WIDTH);
                match orientation {
                    BorderOrientation::Vertical => {
                        Self::stroke_line(ctx, x1 - 1.0, y1, x1 - 1.0, y2);
                        Self::stroke_line(ctx, x1 + 1.0, y1, x1 + 1.0, y2);
                    }
                    BorderOrientation::Horizontal => {
                        Self::stroke_line(ctx, x1, y1 - 1.0, x2, y1 - 1.0);
                        Self::stroke_line(ctx, x1, y1 + 1.0, x2, y1 + 1.0);
                    }
                }
            }
            // Thin, Dotted, SlantDashDot, and anything else → single thin line.
            _ => {
                ctx.set_line_width(STANDARD_BORDER_WIDTH);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
        }
        ctx.restore();
    }

    pub(super) fn stroke_line(
        ctx: &CanvasRenderingContext2d,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    ) {
        ctx.begin_path();
        ctx.move_to(x1, y1);
        ctx.line_to(x2, y2);
        ctx.stroke();
    }
}
