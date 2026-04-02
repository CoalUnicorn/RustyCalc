//! Canvas 2D renderer for the spreadsheet grid.
//!
//! This module is the only piece of RustyCalc that talks to the browser's
//! Canvas 2D API. Everything else — Leptos components, signals, event
//! handlers — lives in `src/components/`. The split is deliberate: Leptos
//! manages reactivity and DOM, but the actual cell grid is a `<canvas>`
//! element drawn imperatively, because HTML tables/divs can't keep up with
//! thousands of cells at 60fps.
//!
//! # How it connects to Leptos
//!
//! The `Worksheet` component (`src/components/worksheet.rs`) owns the
//! `<canvas>` element and holds a `NodeRef` to it. Whenever
//! `state.redraw` (an `RwSignal<u32>`) increments, an `Effect` fires,
//! creates a fresh `CanvasRenderer` from the `NodeRef`, and calls
//! `renderer.render(model, overlays)`. That single call redraws everything.
//!
//! The renderer is intentionally stateless between frames — it's
//! constructed, used, and dropped each redraw. This avoids stale-state
//! bugs: canvas size, DPR, and theme can change between frames.
//!
//! # Render pipeline
//!
//! `render()` runs four phases in order, each building on the previous:
//!
//! ```text
//! Phase 1 — Cell backgrounds and borders
//!   For each of the four frozen-pane quadrants, iterate visible cells.
//!   Paint the fill color, then resolve and draw all four border edges.
//!   Collect text layout (`CellText`) into a Vec for Phase 4.
//!
//! Phase 2 — Row and column headers
//!   Paint the grey header bars with row numbers and column letters (A, B, …).
//!   Selected headers get a highlighted background.
//!
//! Phase 3 — Selection and overlays
//!   Draw the blue selection rectangle, autofill handle, clipboard marching
//!   ants, and point-mode range highlight on top of the cell grid.
//!
//! Phase 4 — Cell text
//!   Paint all collected `CellText` entries last so text always appears
//!   above backgrounds, selection tint, and header lines.
//! ```
//!
//! Text is deferred to Phase 4 because earlier phases may paint over cells
//! (e.g. the selection fill tint covers an area). Drawing text last keeps
//! it readable.
//!
//! # Frozen panes
//!
//! The grid supports frozen rows and columns (Excel's "Freeze Panes").
//! This splits the canvas into up to four quadrants:
//!
//! ```text
//! ┌    ┬      ┐
//! │ frozen/    │ frozen rows,     │
//! │ frozen     │ scrollable cols  │
//! ├    ┼      ┤
//! │ scrollable │ main scrollable  │
//! │ rows,      │ area             │
//! │ frozen cols│                  │
//! └    ┴      ┘
//! ```
//!
//! Each quadrant is rendered by `render_pane()` with different row/col
//! ranges and pixel offsets. A thick separator line marks the freeze
//! boundary.
//!
//! # Border resolution
//!
//! Each cell has four border edges (left, top, right, bottom). The
//! renderer resolves each edge by checking, in order:
//! 1. The cell's own explicit border (from styling)
//! 2. The adjacent neighbour's matching border (left cell's right, etc.)
//! 3. The background color of either cell (for a clean edge between fills)
//! 4. The grid line color (thin grey default)
//!
//! To avoid allocations in this hot path, all colors are borrowed (`&str`)
//! from the style structs or theme — no `String` cloning per cell.
//!
//! # Key types
//!
//! - `CanvasRenderer` — short-lived; created per frame from a canvas element
//! - `CellText` / `TextLine` — pre-computed text layout collected during
//!    Phase 1 and painted in Phase 4
//! - `RenderOverlays` — selection/clipboard/point-mode state passed in from
//!    the Worksheet component each frame
//! - `CanvasTheme` (`src/model/theme.rs`) — static color palette; the Canvas 2D
//!    API can't read CSS variables, so concrete color strings are needed

use ironcalc_base::types::{BorderStyle, HorizontalAlignment, VerticalAlignment};
use ironcalc_base::UserModel;

use crate::model::frontend_model::FrontendModel;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use super::geometry::PixelRect;
use super::geometry::*;
use crate::theme::CanvasTheme;

// Layout constants
const CELL_PADDING: f64 = 4.0;
const DEFAULT_FONT_FAMILY: &str = "Inter, Arial, sans-serif";

// Pre-computed text for one cell

/// One visual line of text inside a cell, positioned for center-aligned rendering.
struct TextLine {
    text: String,
    center_x: f64,
    center_y: f64,
    width: f64,
}

struct CellText {
    /// Clip rectangle — the cell's pixel bounds.
    clip: PixelRect,
    font: String,
    font_size_px: f64,
    text_color: String,
    underlined: bool,
    strike: bool,
    lines: Vec<TextLine>,
}

// Parameter structs

/// Line segment passed to the border-drawing helper.
struct BorderSegment {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

/// Row/column rectangle in sheet coordinates (always normalised: min ≤ max).
struct SheetRange {
    row_min: i32,
    col_min: i32,
    row_max: i32,
    col_max: i32,
}

/// The four index boundaries of the visible (scrollable) area.
#[derive(Copy, Clone, Default)]
struct VisibleRegion {
    /// First scrollable column on screen.
    col_first: i32,
    /// First scrollable row on screen.
    row_first: i32,
    /// Last scrollable column on screen.
    col_last: i32,
    /// Last scrollable row on screen.
    row_last: i32,
}

/// Which outer edges of a cell rect should receive a border stroke.
///
/// Passed to `render_cell_style` so the intent is clear at every call site
/// instead of two anonymous `bool` arguments.
#[derive(Copy, Clone)]
struct CellEdges {
    right: bool,
    bottom: bool,
}

/// Controls whether `draw_dashed_range` fills the interior with a light tint.
#[derive(Copy, Clone, PartialEq, Eq)]
enum DashFill {
    /// Outline only (used for clipboard marching ants).
    Outline,
    /// Outline + semi-transparent fill tint (used for point-mode range).
    Tinted,
}

/// Overlay ranges passed to `render()` for selection preview drawing.
#[derive(Clone, PartialEq)]
pub struct RenderOverlays {
    /// Target cell during autofill-handle drag.
    pub extend_to: Option<(i32, i32)>,
    /// Last Ctrl+C copied range: `(sheet, r1, c1, r2, c2)`.
    pub clipboard: Option<ClipboardRange>,
    /// Range being pointed at during formula entry.
    pub point_range: Option<SheetRect>,
}

/// A copied range on a specific sheet.
#[derive(Copy, Clone, PartialEq)]
pub struct ClipboardRange {
    pub sheet: u32,
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

/// A rectangular region of cells (no sheet — always the current sheet).
#[derive(Copy, Clone, PartialEq)]
pub struct SheetRect {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

// CanvasRenderer

pub struct CanvasRenderer {
    ctx: CanvasRenderingContext2d,
    width: f64,
    height: f64,
    theme: CanvasTheme,
    /// Visible cell bounds — populated at the start of each `render()` call.
    /// Stored on the struct so internal helpers don't need it as a parameter.
    vis: VisibleRegion,
}

impl CanvasRenderer {
    /// Bind a renderer to `canvas` and apply device-pixel-ratio scaling.
    ///
    /// **Performance note:** `canvas.set_width()` / `set_height()` resets the
    /// entire canvas bitmap and all 2D context state — even when the value is
    /// unchanged.  On a 1920×1080 display at 2× DPR that is a ~32 MB backing
    /// store reallocation every frame, which causes >500 ms lag on rapid
    /// navigation (held arrow keys, resize drags).
    ///
    /// Fix: only resize when dimensions actually changed.  When the size is
    /// stable, reset only the transform matrix to the identity before
    /// re-applying the DPR scale.  `clear_rect` in `render()` handles the
    /// pixel clear without touching the backing store.
    #[allow(clippy::expect_used)]
    pub fn new(canvas: &HtmlCanvasElement, theme: CanvasTheme) -> Self {
        let ctx = canvas
            .get_context("2d")
            .expect("getContext should not throw")
            .expect("2d context must exist")
            .unchecked_into::<CanvasRenderingContext2d>();

        let width = canvas.client_width() as f64;
        let height = canvas.client_height() as f64;
        let dpr = web_sys::window()
            .expect("window must exist in WASM context")
            .device_pixel_ratio();

        let target_w = (width * dpr) as u32;
        let target_h = (height * dpr) as u32;

        if canvas.width() != target_w || canvas.height() != target_h {
            // Resize resets canvas bitmap + all context state; necessary here.
            canvas.set_width(target_w);
            canvas.set_height(target_h);
        } else {
            // Reset only the transform so the DPR scale below is applied to
            // the identity matrix, not accumulated across frames.
            ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
                .expect("set_transform should not fail");
        }
        ctx.scale(dpr, dpr).expect("scale should not fail");

        Self {
            ctx,
            width,
            height,
            theme,
            vis: VisibleRegion::default(),
        }
    }

    // Entry point

    /// Full redraw of the spreadsheet canvas.
    pub fn render(&mut self, model: &UserModel, overlays: &RenderOverlays) {
        self.vis = self.visible_cells(model);

        let ctx = &self.ctx;
        ctx.set_line_width(1.0);
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        ctx.clear_rect(0.0, 0.0, self.width, self.height);

        let view = model.get_selected_view();
        let sheet = view.sheet;
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);

        let vis = self.vis;

        let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(model, sheet, r)).sum();
        let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(model, sheet, c)).sum();

        // Cell texts are collected across ALL panes and rendered last (Phase 4)
        // so they always appear on top of backgrounds, selection fill, and headers.
        let mut cell_texts: Vec<CellText> = Vec::new();

        // Phase 1: Cell backgrounds + borders

        // Top-left pane (frozen rows ∩ frozen cols)
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            1..=frozen_rows,
            1..=frozen_cols,
            HEADER_ROW_HEIGHT + 0.5,
            HEADER_COL_WIDTH + 0.5,
            frozen_cols,
            frozen_rows,
        );

        // Frozen separators
        let sep_y = HEADER_ROW_HEIGHT + frozen_rows_h + 0.5;
        let sep_x = HEADER_COL_WIDTH + frozen_cols_w + 0.5;
        let offset = FROZEN_SEP / 2.0;

        if frozen_rows > 0 {
            ctx.set_line_width(FROZEN_SEP);
            ctx.set_stroke_style_str(self.theme.grid_separator_color);
            ctx.begin_path();
            ctx.move_to(0.0, sep_y + offset);
            ctx.line_to(self.width, sep_y + offset);
            ctx.stroke();
            ctx.set_line_width(1.0);
        }
        if frozen_cols > 0 {
            ctx.set_line_width(FROZEN_SEP);
            ctx.set_stroke_style_str(self.theme.grid_separator_color);
            ctx.begin_path();
            ctx.move_to(sep_x + offset, 0.0);
            ctx.line_to(sep_x + offset, self.height);
            ctx.stroke();
            ctx.set_line_width(1.0);
        }

        // Pixel origin of the scrollable (non-frozen) pane.
        let frozen_x =
            HEADER_COL_WIDTH + frozen_cols_w + if frozen_cols > 0 { FROZEN_SEP } else { 0.0 };
        let frozen_y =
            HEADER_ROW_HEIGHT + frozen_rows_h + if frozen_rows > 0 { FROZEN_SEP } else { 0.0 };

        // Top-right pane (frozen rows, scrollable cols)
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            1..=frozen_rows,
            vis.col_first..=vis.col_last,
            HEADER_ROW_HEIGHT + 0.5,
            frozen_x,
            vis.col_last,
            frozen_rows,
        );

        // Bottom-left pane (scrollable rows, frozen cols)
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            vis.row_first..=vis.row_last,
            1..=frozen_cols,
            frozen_y,
            HEADER_COL_WIDTH + 0.5,
            frozen_cols,
            vis.row_last,
        );

        // Bottom-right pane (main scrollable area)
        self.render_pane(
            model,
            sheet,
            &mut cell_texts,
            vis.row_first..=vis.row_last,
            vis.col_first..=vis.col_last,
            frozen_y,
            frozen_x,
            vis.col_last,
            vis.row_last,
        );

        // Phase 2: Headers + corner box
        self.render_row_headers(model, sheet, frozen_rows, frozen_y);
        self.render_column_headers(model, sheet, frozen_cols, frozen_x);

        // Corner box (top-left blank square)
        ctx.set_fill_style_str(self.theme.header_bg);
        ctx.fill_rect(0.0, 0.0, HEADER_COL_WIDTH, HEADER_ROW_HEIGHT);
        ctx.set_stroke_style_str(self.theme.header_border_color);
        ctx.set_line_width(1.0);
        ctx.begin_path();
        ctx.move_to(0.0, HEADER_ROW_HEIGHT + 0.5);
        ctx.line_to(self.width, HEADER_ROW_HEIGHT + 0.5);
        ctx.stroke();
        ctx.begin_path();
        ctx.move_to(HEADER_COL_WIDTH + 0.5, 0.0);
        ctx.line_to(HEADER_COL_WIDTH + 0.5, self.height);
        ctx.stroke();

        // Phase 3: Selection outline
        self.draw_selection(model, sheet, frozen_x, frozen_y);
        if let Some((to_row, to_col)) = overlays.extend_to {
            self.draw_extend_preview(model, sheet, frozen_x, frozen_y, to_row, to_col);
        }

        // Marching-ants border around the last Ctrl+C copied range.
        if let Some(ref cb) = overlays.clipboard {
            if cb.sheet == sheet {
                self.draw_dashed_range(
                    model,
                    sheet,
                    frozen_x,
                    frozen_y,
                    SheetRange {
                        row_min: cb.r1.min(cb.r2),
                        col_min: cb.c1.min(cb.c2),
                        row_max: cb.r1.max(cb.r2),
                        col_max: cb.c1.max(cb.c2),
                    },
                    self.theme.selection_color,
                    DashFill::Outline,
                );
            }
        }

        // Point-mode range: blue dashed outline + light fill tint.
        if let Some(ref pr) = overlays.point_range {
            self.draw_dashed_range(
                model,
                sheet,
                frozen_x,
                frozen_y,
                SheetRange {
                    row_min: pr.r1.min(pr.r2),
                    col_min: pr.c1.min(pr.c2),
                    row_max: pr.r1.max(pr.r2),
                    col_max: pr.c1.max(pr.c2),
                },
                "#1E6FD9",
                DashFill::Tinted,
            );
        }

        // Phase 4: Cell text — always on top
        // Rendered after selection fill so text is readable over the blue tint,
        // and after the active-cell white-fill so text appears on a clean background.
        ctx.set_text_align("center");
        ctx.set_text_baseline("middle");
        for ct in &cell_texts {
            self.render_cell_text(ct);
        }
    }

    // Pane helper (DRYs the four frozen-pane quadrants)

    // TODO: Refactor
    /// Render cell backgrounds, borders, and collect text for one pane quadrant.
    fn render_pane(
        &self,
        model: &UserModel,
        sheet: u32,
        cell_texts: &mut Vec<CellText>,
        rows: std::ops::RangeInclusive<i32>,
        cols: std::ops::RangeInclusive<i32>,
        start_y: f64,
        start_x: f64,
        last_col: i32,
        last_row: i32,
    ) {
        let mut y = start_y;
        for row in rows {
            let rh = row_height(model, sheet, row);
            let mut x = start_x;
            for col in cols.clone() {
                let cw = col_width(model, sheet, col);
                let rect = PixelRect {
                    x,
                    y,
                    width: cw,
                    height: rh,
                };
                self.render_cell_style(
                    model,
                    sheet,
                    row,
                    col,
                    rect,
                    CellEdges {
                        right: col == last_col,
                        bottom: row == last_row,
                    },
                );
                if let Some(ct) = self.compute_cell_text(model, sheet, row, col, rect) {
                    cell_texts.push(ct);
                }
                x += cw;
            }
            y += rh;
        }
    }

    // Cell style (background + left/top borders)
    // TODO: Refactor

    fn render_cell_style(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        col: i32,
        rect: PixelRect,
        edges: CellEdges,
    ) {
        let PixelRect {
            x,
            y,
            width: w,
            height: h,
        } = rect;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let ctx = &self.ctx;
        let style = model.get_cell_style(sheet, row, col).unwrap_or_default();
        let show_grid = model.get_show_grid_lines(sheet).unwrap_or(true);

        let bg: &str = style.fill.fg_color.as_deref().unwrap_or(self.theme.cell_bg);
        let cell_grid_color: &str = if show_grid { self.theme.grid_color } else { bg };

        ctx.set_fill_style_str(bg);
        ctx.fill_rect(x, y, w, h);

        // Left border: use this cell's left, or neighbour's right, or grid color.
        // Fetch left neighbour's style once — its lifetime must span the match.
        let left_nb = if col > 1 && style.border.left.is_none() {
            Some(
                model
                    .get_cell_style(sheet, row, col - 1)
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let (bl_color, bl_style) = if let Some(ref bl) = style.border.left {
            (bl.color.as_deref().unwrap_or(cell_grid_color), &bl.style)
        } else if let Some(ref left) = left_nb {
            if let Some(ref br) = left.border.right {
                (br.color.as_deref().unwrap_or(cell_grid_color), &br.style)
            } else if style.fill.fg_color.is_some() {
                (bg, &BorderStyle::Thin)
            } else if let Some(ref nbg) = left.fill.fg_color {
                (nbg.as_str(), &BorderStyle::Thin)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            }
        } else {
            // Column 1 or has explicit left border (handled above).
            if style.fill.fg_color.is_some() {
                (bg, &BorderStyle::Thin)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            }
        };
        self.draw_border(
            ctx,
            bl_style,
            bl_color,
            BorderSegment {
                x1: x,
                y1: y,
                x2: x,
                y2: y + h,
            },
            true,
        );

        // Top border: use this cell's top, or neighbour's bottom, or grid color.
        let top_nb = if row > 1 && style.border.top.is_none() {
            Some(
                model
                    .get_cell_style(sheet, row - 1, col)
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let (bt_color, bt_style) = if let Some(ref bt) = style.border.top {
            (bt.color.as_deref().unwrap_or(cell_grid_color), &bt.style)
        } else if let Some(ref top) = top_nb {
            if let Some(ref bb) = top.border.bottom {
                (bb.color.as_deref().unwrap_or(cell_grid_color), &bb.style)
            } else if style.fill.fg_color.is_some() {
                (bg, &BorderStyle::Thin)
            } else if let Some(ref nbg) = top.fill.fg_color {
                (nbg.as_str(), &BorderStyle::Thin)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            }
        } else {
            // Row 1 or has explicit top border (handled above).
            if style.fill.fg_color.is_some() {
                (bg, &BorderStyle::Thin)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            }
        };
        self.draw_border(
            ctx,
            bt_style,
            bt_color,
            BorderSegment {
                x1: x,
                y1: y,
                x2: x + w,
                y2: y,
            },
            false,
        );

        // Right border: always draw when the cell has an explicit right border style;
        // also draw the grid line for the rightmost visible column in each pane.
        let draw_right_explicit = style.border.right.is_some();
        if edges.right || draw_right_explicit {
            let (br_color, br_style) = if let Some(ref br) = style.border.right {
                (br.color.as_deref().unwrap_or(cell_grid_color), &br.style)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            };
            self.draw_border(
                ctx,
                br_style,
                br_color,
                BorderSegment {
                    x1: x + w,
                    y1: y,
                    x2: x + w,
                    y2: y + h,
                },
                true,
            );
        }

        // Bottom border: always draw when the cell has an explicit bottom border style;
        // also draw the grid line for the bottommost visible row in each pane.
        let draw_bottom_explicit = style.border.bottom.is_some();
        if edges.bottom || draw_bottom_explicit {
            let (bb_color, bb_style) = if let Some(ref bb) = style.border.bottom {
                (bb.color.as_deref().unwrap_or(cell_grid_color), &bb.style)
            } else {
                (cell_grid_color, &BorderStyle::Thin)
            };
            self.draw_border(
                ctx,
                bb_style,
                bb_color,
                BorderSegment {
                    x1: x,
                    y1: y + h,
                    x2: x + w,
                    y2: y + h,
                },
                false,
            );
        }
    }

    // Border helper

    fn draw_border(
        &self,
        ctx: &CanvasRenderingContext2d,
        style: &BorderStyle,
        color: &str,
        seg: BorderSegment,
        is_vertical: bool,
    ) {
        let BorderSegment { x1, y1, x2, y2 } = seg;
        ctx.save();
        ctx.set_stroke_style_str(color);
        match style {
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot => {
                ctx.set_line_width(2.0);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
            BorderStyle::Thick => {
                ctx.set_line_width(3.0);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
            BorderStyle::Double => {
                ctx.set_line_width(1.0);
                if is_vertical {
                    Self::stroke_line(ctx, x1 - 1.0, y1, x1 - 1.0, y2);
                    Self::stroke_line(ctx, x1 + 1.0, y1, x1 + 1.0, y2);
                } else {
                    Self::stroke_line(ctx, x1, y1 - 1.0, x2, y1 - 1.0);
                    Self::stroke_line(ctx, x1, y1 + 1.0, x2, y1 + 1.0);
                }
            }
            // Thin, Dotted, SlantDashDot, and anything else -> single thin line.
            // TODO: implement dash patterns for Dotted/SlantDashDot with setLineDash.
            _ => {
                ctx.set_line_width(1.0);
                Self::stroke_line(ctx, x1, y1, x2, y2);
            }
        }
        ctx.restore();
    }

    fn stroke_line(ctx: &CanvasRenderingContext2d, x1: f64, y1: f64, x2: f64, y2: f64) {
        ctx.begin_path();
        ctx.move_to(x1, y1);
        ctx.line_to(x2, y2);
        ctx.stroke();
    }

    // Cell text layout + paint

    /// Build the text layout for a cell; returns `None` for empty cells.
    fn compute_cell_text(
        &self,
        model: &UserModel,
        sheet: u32,
        row: i32,
        col: i32,
        rect: PixelRect,
    ) -> Option<CellText> {
        let PixelRect {
            x,
            y,
            width,
            height,
        } = rect;
        if width <= 0.0 || height <= 0.0 {
            return None;
        }
        let text = model
            .get_formatted_cell_value(sheet, row, col)
            .unwrap_or_default();
        if text.is_empty() {
            return None;
        }

        let resolved = model.cell_style(sheet, row, col, self.theme.default_text_color);
        let font = resolved.font.css.clone();
        let font_size = resolved.font.size_px;
        let text_color = resolved.text_color.as_str().to_owned();
        let effective_h_align = resolved.h_align;
        let effective_v_align = resolved.v_align;

        let approx_char_w = font_size * 0.6;
        let line_height = font_size * 1.5;
        let usable_w = width - 2.0 * CELL_PADDING;
        let wrap = resolved.wrap_text;

        // Set font on ctx now so measure_text() returns accurate widths.
        self.ctx.set_font(&font);

        // Build the list of visual lines, optionally word-wrapping.
        let text_lines: Vec<String> = if wrap && usable_w > 0.0 {
            let mut result: Vec<String> = Vec::new();
            for raw_line in text.split('\n') {
                let mut current = String::new();
                for word in raw_line.split_whitespace() {
                    let candidate = if current.is_empty() {
                        word.to_owned()
                    } else {
                        format!("{current} {word}")
                    };
                    let w = self
                        .ctx
                        .measure_text(&candidate)
                        .map(|m| m.width())
                        .unwrap_or(candidate.len() as f64 * approx_char_w);
                    if w <= usable_w || current.is_empty() {
                        current = candidate;
                    } else {
                        result.push(current);
                        current = word.to_owned();
                    }
                }
                result.push(current);
            }
            result
        } else {
            text.split('\n').map(str::to_owned).collect()
        };

        let line_count = text_lines.len() as f64;
        let mut lines: Vec<TextLine> = Vec::new();

        for (i, line) in text_lines.iter().enumerate() {
            let tw = self
                .ctx
                .measure_text(line)
                .map(|m| m.width())
                .unwrap_or(line.len() as f64 * approx_char_w);
            let i_f = i as f64;
            let center_x = match effective_h_align {
                HorizontalAlignment::Right => width - CELL_PADDING + x - tw / 2.0,
                HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                    x + width / 2.0
                }
                _ => CELL_PADDING + x + tw / 2.0,
            };
            let center_y = match effective_v_align {
                VerticalAlignment::Bottom => {
                    y + height - font_size / 2.0 - 4.0 + (i_f - line_count + 1.0) * line_height
                }
                VerticalAlignment::Center => {
                    y + height / 2.0 + (i_f + (1.0 - line_count) / 2.0) * line_height
                }
                _ => y + font_size / 2.0 + 4.0 + i_f * line_height,
            };
            lines.push(TextLine {
                text: line.clone(),
                center_x,
                center_y,
                width: tw,
            });
        }

        Some(CellText {
            clip: PixelRect {
                x,
                y,
                width,
                height,
            },
            font,
            font_size_px: font_size,
            text_color,
            underlined: resolved.font.underline,
            strike: resolved.font.strikethrough,
            lines,
        })
    }

    /// Paint a pre-computed `CellText` onto the canvas.
    fn render_cell_text(&self, ct: &CellText) {
        let ctx = &self.ctx;
        ctx.set_font(&ct.font);
        ctx.set_fill_style_str(&ct.text_color);

        ctx.save();
        ctx.begin_path();
        ctx.rect(ct.clip.x, ct.clip.y, ct.clip.width, ct.clip.height);
        ctx.clip();

        for line in &ct.lines {
            ctx.fill_text(&line.text, line.center_x, line.center_y).ok();
            if ct.underlined {
                let underline_offset = (ct.font_size_px * 0.12).max(2.0);
                ctx.begin_path();
                ctx.set_stroke_style_str(&ct.text_color);
                ctx.set_line_width(1.0);
                ctx.move_to(
                    line.center_x - line.width / 2.0,
                    line.center_y + underline_offset,
                );
                ctx.line_to(
                    line.center_x + line.width / 2.0,
                    line.center_y + underline_offset,
                );
                ctx.stroke();
            }
            if ct.strike {
                ctx.begin_path();
                ctx.set_stroke_style_str(&ct.text_color);
                ctx.set_line_width(1.0);
                ctx.move_to(line.center_x - line.width / 2.0, line.center_y);
                ctx.line_to(line.center_x + line.width / 2.0, line.center_y);
                ctx.stroke();
            }
        }
        ctx.restore();
    }

    // Row headers

    fn render_row_headers(&self, model: &UserModel, sheet: u32, frozen_rows: i32, frozen_y: f64) {
        let ctx = &self.ctx;
        let view = model.get_selected_view();
        let sel_row_start = view.range[0].min(view.range[2]);
        let sel_row_end = view.range[0].max(view.range[2]);

        ctx.set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        let first_row = if frozen_rows == 0 {
            self.vis.row_first
        } else {
            1
        };
        let mut top_y = if first_row == 1 {
            HEADER_ROW_HEIGHT + 0.5
        } else {
            frozen_y
        };

        let mut row = first_row;
        loop {
            if row > self.vis.row_last {
                break;
            }
            let rh = row_height(model, sheet, row);
            if rh > 0.0 {
                let selected = row >= sel_row_start && row <= sel_row_end;
                ctx.set_fill_style_str(self.theme.header_border_color);
                ctx.fill_rect(0.5, top_y, HEADER_COL_WIDTH, rh);
                ctx.set_fill_style_str(if selected {
                    self.theme.header_selected_bg
                } else {
                    self.theme.header_bg
                });
                ctx.fill_rect(0.5, top_y + 0.5, HEADER_COL_WIDTH, rh - 1.0);
                ctx.set_fill_style_str(if selected {
                    self.theme.header_selected_color
                } else {
                    self.theme.header_text_color
                });
                ctx.fill_text(&row.to_string(), HEADER_COL_WIDTH / 2.0, top_y + rh / 2.0)
                    .ok();
                top_y += rh;
            }
            if row == frozen_rows {
                top_y = frozen_y;
                row = self.vis.row_first;
            } else {
                row += 1;
            }
        }
    }

    // Column headers

    fn render_column_headers(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_cols: i32,
        frozen_x: f64,
    ) {
        let ctx = &self.ctx;
        let view = model.get_selected_view();
        let sel_col_start = view.range[1].min(view.range[3]);
        let sel_col_end = view.range[1].max(view.range[3]);

        ctx.set_font(&format!("bold 12px {DEFAULT_FONT_FAMILY}"));

        // Frozen columns strip
        let mut x = HEADER_COL_WIDTH + 0.5;
        for col in 1..=frozen_cols {
            let cw = col_width(model, sheet, col);
            self.draw_col_header(ctx, col, x, cw, sel_col_start, sel_col_end);
            x += cw;
        }

        // Scrollable columns strip
        let mut x = if frozen_cols > 0 {
            frozen_x
        } else {
            HEADER_COL_WIDTH + 0.5
        };
        for col in self.vis.col_first..=self.vis.col_last {
            let cw = col_width(model, sheet, col);
            self.draw_col_header(ctx, col, x, cw, sel_col_start, sel_col_end);
            x += cw;
        }
    }

    fn draw_col_header(
        &self,
        ctx: &CanvasRenderingContext2d,
        col: i32,
        x: f64,
        cw: f64,
        sel_col_start: i32,
        sel_col_end: i32,
    ) {
        let selected = col >= sel_col_start && col <= sel_col_end;
        ctx.set_fill_style_str(self.theme.header_border_color);
        ctx.fill_rect(x, 0.5, cw, HEADER_ROW_HEIGHT);
        ctx.set_fill_style_str(if selected {
            self.theme.header_selected_bg
        } else {
            self.theme.header_bg
        });
        ctx.fill_rect(x + 0.5, 0.5, cw - 1.0, HEADER_ROW_HEIGHT);
        ctx.set_fill_style_str(if selected {
            self.theme.header_selected_color
        } else {
            self.theme.header_text_color
        });
        ctx.fill_text(&col_name(col), x + cw / 2.0, HEADER_ROW_HEIGHT / 2.0)
            .ok();
    }

    // Selection outline

    /// Draw the blue selection border directly on canvas.
    fn draw_selection(&self, model: &UserModel, sheet: u32, frozen_x: f64, frozen_y: f64) {
        let view = model.get_selected_view();
        let [r1, c1, r2, c2] = view.range;
        let (r_min, r_max) = (r1.min(r2), r1.max(r2));
        let (c_min, c_max) = (c1.min(c2), c1.max(c2));

        let x1 = self.cell_x(model, sheet, c_min, frozen_x);
        let y1 = self.cell_y(model, sheet, r_min, frozen_y);

        // Clamp to the last visible column/row to avoid O(MAX_COLS/MAX_ROWS) iteration
        // in cell_x/cell_y when a full row or column is selected. The canvas clips
        // anything past its edge anyway, so the visual result is identical.
        let x2 = if c_max > self.vis.col_last {
            self.width
        } else {
            self.cell_x(model, sheet, c_max, frozen_x) + col_width(model, sheet, c_max)
        };
        let y2 = if r_max > self.vis.row_last {
            self.height
        } else {
            self.cell_y(model, sheet, r_max, frozen_y) + row_height(model, sheet, r_max)
        };

        let w = x2 - x1;
        let h = y2 - y1;

        let ctx = &self.ctx;

        // Semi-transparent fill over the entire range
        ctx.set_fill_style_str(self.theme.selection_fill);
        ctx.fill_rect(x1, y1, w, h);

        // Restore the active cell's actual fill color and borders so they
        // remain visible while the cell is selected.  Phase 4 re-renders all
        // text on top, so we only need to restore the visual style here.
        let ax = self.cell_x(model, sheet, view.column, frozen_x);
        let ay = self.cell_y(model, sheet, view.row, frozen_y);
        let aw = col_width(model, sheet, view.column);
        let ah = row_height(model, sheet, view.row);
        self.render_cell_style(
            model,
            sheet,
            view.row,
            view.column,
            PixelRect {
                x: ax,
                y: ay,
                width: aw,
                height: ah,
            },
            CellEdges {
                right: true,
                bottom: true,
            },
        );

        // 2px border around the full selection range
        ctx.set_stroke_style_str(self.theme.selection_color);
        ctx.set_line_width(2.0);
        ctx.stroke_rect(x1, y1, w, h);
        ctx.set_line_width(1.0);

        // Autofill handle: solid 6×6 square at bottom-right corner of range
        let hx = x2 - (AUTOFILL_HANDLE_PX / 2.0);
        let hy = y2 - (AUTOFILL_HANDLE_PX / 2.0);
        ctx.set_fill_style_str(self.theme.selection_color);
        ctx.fill_rect(hx, hy, AUTOFILL_HANDLE_PX, AUTOFILL_HANDLE_PX);
    }

    /// Draw a dashed preview border over the area that would be filled if the
    /// user releases the autofill handle at `(to_row, to_col)`.
    fn draw_extend_preview(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_x: f64,
        frozen_y: f64,
        to_row: i32,
        to_col: i32,
    ) {
        let view = model.get_selected_view();
        let [r1, c1, r2, c2] = view.range;
        let (r_min, r_max) = (r1.min(r2).min(to_row), r1.max(r2).max(to_row));
        let (c_min, c_max) = (c1.min(c2).min(to_col), c1.max(c2).max(to_col));

        let x1 = self.cell_x(model, sheet, c_min, frozen_x);
        let y1 = self.cell_y(model, sheet, r_min, frozen_y);
        let x2 = self.cell_x(model, sheet, c_max, frozen_x) + col_width(model, sheet, c_max);
        let y2 = self.cell_y(model, sheet, r_max, frozen_y) + row_height(model, sheet, r_max);

        let ctx = &self.ctx;
        let dash = web_sys::js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into());
        ctx.set_line_dash(&dash).ok();
        ctx.set_stroke_style_str(self.theme.selection_color);
        ctx.set_line_width(1.0);
        ctx.stroke_rect(x1, y1, x2 - x1, y2 - y1);
        ctx.set_line_dash(&web_sys::js_sys::Array::new()).ok();
    }

    /// Draw a dashed border rectangle covering the cell range `(r1,c1)-(r2,c2)`.
    ///
    /// Used for both marching-ants (clipboard) and point-mode overlays.
    /// When `fill_tint` is `true`, a 10%-opacity fill of `color` is also drawn.
    fn draw_dashed_range(
        &self,
        model: &UserModel,
        sheet: u32,
        frozen_x: f64,
        frozen_y: f64,
        range: SheetRange,
        color: &str,
        fill: DashFill,
    ) {
        let x1 = self.cell_x(model, sheet, range.col_min, frozen_x);
        let y1 = self.cell_y(model, sheet, range.row_min, frozen_y);
        let x2 = if range.col_max > self.vis.col_last {
            self.width
        } else {
            self.cell_x(model, sheet, range.col_max, frozen_x)
                + col_width(model, sheet, range.col_max)
        };
        let y2 = if range.row_max > self.vis.row_last {
            self.height
        } else {
            self.cell_y(model, sheet, range.row_max, frozen_y)
                + row_height(model, sheet, range.row_max)
        };

        let ctx = &self.ctx;
        let dash = web_sys::js_sys::Array::of2(&4.0_f64.into(), &3.0_f64.into());
        ctx.set_line_dash(&dash).ok();
        ctx.set_stroke_style_str(color);
        ctx.set_line_width(1.5);
        ctx.stroke_rect(x1, y1, x2 - x1, y2 - y1);
        ctx.set_line_dash(&web_sys::js_sys::Array::new()).ok();
        ctx.set_line_width(1.0);

        if fill == DashFill::Tinted {
            // Build "rgba(r,g,b,0.08)" from a hex color — only handles 6-digit hex.
            let tint = hex_to_rgba(color, 0.08);
            ctx.set_fill_style_str(&tint);
            ctx.fill_rect(x1, y1, x2 - x1, y2 - y1);
        }
    }

    // Coordinate helpers

    fn cell_x(&self, model: &UserModel, sheet: u32, col: i32, frozen_x: f64) -> f64 {
        let view = model.get_selected_view();
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        if col <= frozen_cols {
            return HEADER_COL_WIDTH
                + 0.5
                + (1..col).map(|c| col_width(model, sheet, c)).sum::<f64>();
        }
        let left_col = view.left_column.max(frozen_cols + 1);
        frozen_x
            + (left_col..col)
                .map(|c| col_width(model, sheet, c))
                .sum::<f64>()
    }

    fn cell_y(&self, model: &UserModel, sheet: u32, row: i32, frozen_y: f64) -> f64 {
        let view = model.get_selected_view();
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        if row <= frozen_rows {
            return HEADER_ROW_HEIGHT
                + 0.5
                + (1..row).map(|r| row_height(model, sheet, r)).sum::<f64>();
        }
        let top_row = view.top_row.max(frozen_rows + 1);
        frozen_y
            + (top_row..row)
                .map(|r| row_height(model, sheet, r))
                .sum::<f64>()
    }

    /// Compute the visible (scrollable) cell region.
    fn visible_cells(&self, model: &UserModel) -> VisibleRegion {
        let view = model.get_selected_view();
        let sheet = view.sheet;
        let frozen_rows = model.get_frozen_rows_count(sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(sheet).unwrap_or(0);
        let frozen_rows_h: f64 = (1..=frozen_rows).map(|r| row_height(model, sheet, r)).sum();
        let frozen_cols_w: f64 = (1..=frozen_cols).map(|c| col_width(model, sheet, c)).sum();

        let row_first = (frozen_rows + 1).max(view.top_row);
        let col_first = (frozen_cols + 1).max(view.left_column);

        let mut row_last = row_first;
        let mut y = HEADER_ROW_HEIGHT + frozen_rows_h;
        for row in row_first..=LAST_ROW {
            if y >= self.height || row == LAST_ROW {
                row_last = row;
                break;
            }
            y += row_height(model, sheet, row);
        }

        let mut col_last = col_first;
        let mut x = HEADER_COL_WIDTH + frozen_cols_w;
        for col in col_first..=LAST_COLUMN {
            if x >= self.width || col == LAST_COLUMN {
                col_last = col;
                break;
            }
            x += col_width(model, sheet, col);
        }

        VisibleRegion {
            col_first,
            row_first,
            col_last,
            row_last,
        }
    }
}

// Free helpers

/// Convert a 6-digit hex color (`"#1E6FD9"`) to an `rgba(…)` CSS string with
/// the given alpha.  Falls back to transparent on malformed input.
fn hex_to_rgba(hex: &str, alpha: f64) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return format!("rgba(0,0,0,{alpha})");
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    format!("rgba({r},{g},{b},{alpha})")
}

// col_name() lives in canvas::geometry and is imported at the top of this file.
