//! Cell painting - pure pixel pusher.
//!
//! `render_pane` iterates the resolved-paint stream from `CellPaintsIter`
//! and hands each `CellPaint` to `paint_cell`. Nothing in this file talks
//! to the model: bg, borders, and text are pre-resolved upstream.

use std::ops::RangeInclusive;

use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{BorderEdge, Point};
use crate::geometry::utils::{col_width, row_height};
use crate::renderer::pane::PaneRegion;
use crate::renderer::text_paint::TextPaint;
use crate::theme::CanvasTheme;
use crate::types::coord::CssColor;
use crate::{CanvasModel, CanvasSize};

use super::super::geometry::frame::FrameContext;
use super::super::types::coord::{CellAddress, RCRange};
use super::CanvasRenderer;
use crate::geometry::constants::{MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

use ironcalc_base::types::{BorderItem, BorderStyle, Style};

impl CanvasRenderer {
    /// Walk one frozen-pane quadrant in four deferred passes:
    /// bg -> grid borders -> explicit borders -> text.
    ///
    /// `BorderEdge::Right`/`Bottom` strokes at `x+width` snap (via
    /// `snap_stroke`) into the NEXT cell's pixel column, where they'd land
    /// inside that neighbour's bg. So this cell can only safely paint a
    /// 1 px stroke on its OWN territory — i.e. its left and top edges
    /// (which snap onto the cell's first column / first row). The grid
    /// fallback therefore lives on left+top only and is suppressed when
    /// the cell carries an explicit fill — colored cells extend cleanly to
    /// every boundary, matching Excel/Sheets (image 5).
    ///
    /// The grid sub-pass runs across all cells before the explicit-border
    /// sub-pass so an explicit `BorderItem::right` on cell A wins over
    /// cell B's grid left at the shared pixel column (paint order: grid
    /// across all → explicit across all → A.right strokes last on the
    /// shared edge). Text remains the final pass so overflow is never
    /// clipped by a neighbour's bg.
    pub(super) fn render_pane(&self, model: &dyn CanvasModel, pane: PaneRegion) {
        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();
        for p in self.paints_in(model, &pane) {
            self.paint_bg(&p);
            slots.push(p);
        }
        for p in &slots {
            self.paint_borders_grid(p);
        }
        for p in &slots {
            self.paint_borders_explicit(p);
        }
        for p in &slots {
            if let Some(tp) = TextPaint::resolve(self, model, p.addr, p.rect, &p.style) {
                self.paint_text(&tp);
            }
        }
        self.frame_cache.text_slots.set(slots);
    }

    /// Fill a cell's background rectangle. Border pass is separate (batched).
    pub(super) fn paint_bg(&self, p: &CellPaint) {
        // Theme-fallback path uses set_fill_static so the pointer-eq fast path
        // in CachedColor::matches_static fires when the same theme color
        // repeats across cells.
        match p.style.fill.fg_color.as_deref() {
            Some(c) => self.set_fill_cached(c),
            None => self.set_fill_static(self.theme().cell_bg),
        }
        self.ctx_ref().fill_rect(
            p.rect.top_left.x,
            p.rect.top_left.y,
            p.rect.width,
            p.rect.height,
        );
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead; the main pane pass uses `paint_borders`.
    pub(super) fn paint_cell(&self, p: &CellPaint) {
        self.paint_bg(p);
        self.paint_borders(p);
    }

    /// Grid-fallback strokes on left+top
    fn paint_borders_grid(&self, p: &CellPaint) {
        if !self.frame_cache.show_grid.get() {
            return;
        }
        if p.style.fill.fg_color.is_some() {
            return;
        }
        let theme = self.theme();
        let b = &p.style.border;
        if b.left.is_none() {
            self.paint_border(BorderEdge::Left, p.rect, &BorderPaint::grid_line(theme));
        }
        if b.top.is_none() {
            self.paint_border(BorderEdge::Top, p.rect, &BorderPaint::grid_line(theme));
        }
    }

    /// Stroke any explicit `BorderItem`s on the cell's four edges. Run
    /// across every slot AFTER the grid sub-pass so an explicit right on
    /// cell A wins over cell B's grid left at the shared pixel column.
    fn paint_borders_explicit(&self, p: &CellPaint) {
        let theme = self.theme();
        let b = &p.style.border;
        if let Some(item) = &b.left {
            self.paint_border(BorderEdge::Left, p.rect, &BorderPaint::resolve(item, theme));
        }
        if let Some(item) = &b.top {
            self.paint_border(BorderEdge::Top, p.rect, &BorderPaint::resolve(item, theme));
        }
        if let Some(item) = &b.right {
            self.paint_border(
                BorderEdge::Right,
                p.rect,
                &BorderPaint::resolve(item, theme),
            );
        }
        if let Some(item) = &b.bottom {
            self.paint_border(
                BorderEdge::Bottom,
                p.rect,
                &BorderPaint::resolve(item, theme),
            );
        }
    }

    /// Single-cell border paint used by `repaint_active_cell` where there
    /// are no neighbour interactions to worry about. Composes the two
    /// sub-passes in their canonical order: grid fallback first, explicit
    /// over the top.
    pub(super) fn paint_borders(&self, p: &CellPaint) {
        self.paint_borders_grid(p);
        self.paint_borders_explicit(p);
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
        match &b.color {
            BorderColor::Static(s) => self.set_stroke_static(s),
            BorderColor::Owned(s) => self.set_stroke_cached(s),
        }
        self.set_line_width_cached(b.stroke.width_px);
        for &d in offsets {
            self.stroke_line(line.offset_cross(d));
        }
    }

    /// Repaint one cell's full paint (bg + borders + text).
    ///
    /// Used by the selection overlay to restore the active cell on top of
    /// the semi-transparent selection fill.
    pub(super) fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        addr: CellAddress,
        frame: &FrameContext,
    ) {
        let range = RCRange::from_cell(addr.row, addr.column);
        let Some(rect) = self.range_pixel_bounds(frame, range) else {
            return;
        };
        let Ok(own_style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };
        let Some(paint) = CellPaint::resolve_cell_paint(self, CellSlot { addr, rect }, own_style)
        else {
            return;
        };
        self.paint_cell(&paint);
        if let Some(t) = TextPaint::resolve(self, model, addr, rect, &paint.style) {
            self.paint_text(&t);
        }
    }
}

//  Cell paint resolution
pub(crate) struct CellPaint {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub style: Style,
}

impl CellPaint {
    /// Resolve one cell Style into renderer-ready `CellPaint`.
    /// Takes Style by value — the caller's owned copy is moved straight through
    /// to the paint, eliminating the per-cell clone on the hot pane walk.
    pub fn resolve_cell_paint(
        _renderer: &CanvasRenderer,
        slot: CellSlot,
        own_style: Style,
    ) -> Option<CellPaint> {
        if slot.rect.width <= 0.0 || slot.rect.height <= 0.0 {
            return None;
        }
        Some(CellPaint {
            addr: slot.addr,
            rect: slot.rect,
            style: own_style,
        })
    }
}

pub(crate) struct BorderStroke {
    pub width_px: f64,
    pub double: bool, // double-line styles render as two parallel strokes
}

/// Color for a resolved border edge. `Static` avoids any allocation for the
/// common grid-line base coat (theme color is `&'static str`); `Owned` carries
/// a dynamic color built from the cell's explicit `BorderItem`.
pub(crate) enum BorderColor {
    Static(&'static str),
    Owned(String),
}

pub(crate) struct BorderPaint {
    pub color: BorderColor,
    pub stroke: BorderStroke,
}

impl BorderPaint {
    /// Thin grid-color stroke used as the right/bottom fallback when a cell
    /// has no explicit border on that edge. Zero alloc — `theme.grid_color`
    /// is `&'static str` so the `Static` arm avoids any per-cell `String`.
    fn grid_line(theme: &CanvasTheme) -> Self {
        Self {
            color: BorderColor::Static(theme.grid_color),
            stroke: BorderStroke {
                width_px: STANDARD_BORDER_WIDTH,
                double: false,
            },
        }
    }

    /// Resolve a `BorderItem` from the cell style into a renderer-ready paint.
    /// Color falls back to `theme.grid_color` when the item carries no explicit color.
    fn resolve(item: &BorderItem, theme: &CanvasTheme) -> Self {
        Self {
            color: BorderColor::Owned(
                CssColor::new(item.color.as_deref().unwrap_or(theme.grid_color)).into_string(),
            ),
            stroke: BorderStroke::from_border_style(&item.style),
        }
    }
}

impl BorderStroke {
    /// Map `BorderStyle` → pixel width + double-line flag.
    /// Dashed/dotted patterns degrade to solid 1 px in v1.
    fn from_border_style(s: &BorderStyle) -> Self {
        match s {
            BorderStyle::Medium
            | BorderStyle::MediumDashed
            | BorderStyle::MediumDashDot
            | BorderStyle::MediumDashDotDot => Self {
                width_px: MEDIUM_BORDER_WIDTH,
                double: false,
            },
            BorderStyle::Thick => Self {
                width_px: THICK_BORDER_WIDTH,
                double: false,
            },
            BorderStyle::Double => Self {
                width_px: STANDARD_BORDER_WIDTH,
                double: true,
            },
            // Thin / Dotted / SlantDashDot / etc.
            _ => Self {
                width_px: STANDARD_BORDER_WIDTH,
                double: false,
            },
        }
    }
}

/// One cell yielded by a `PaneCells` walk: the address, its pixel rect at
/// the current scroll, and any outer pane-boundary borders the renderer
/// must force-draw on it.
#[derive(Clone, Copy)]
pub(crate) struct CellSlot {
    pub addr: CellAddress,
    pub rect: PixelRect,
}

#[derive(Clone)]
pub(crate) struct RowStrip {
    row: i32,
    height: f64,
}

/// Stateful walk over the cells of a `PaneRegion`. Caches per-pane column
/// widths once, threads a row-top accumulator across rows, and skips
/// hidden rows/columns as well as cells that fall off the canvas. Replaces
/// the parameter cluster that used to feed `render_pane_row`.
pub(crate) struct PaneCells<'a> {
    pub pane: &'a PaneRegion,
    pub model: &'a dyn CanvasModel,
    pub sheet: u32,
    pub canvas: CanvasSize,
    pub current_row: Option<RowStrip>,
    pub row_iter: RangeInclusive<i32>,
    pub row_top: f64,
    pub col_iter: RangeInclusive<i32>,
    pub col_left: f64,
}

impl<'a> Iterator for PaneCells<'a> {
    type Item = CellSlot;

    fn next(&mut self) -> Option<CellSlot> {
        loop {
            if self.current_row.is_none() {
                if self.row_top >= self.canvas.h {
                    return None;
                }
                let row = self.row_iter.next()?;
                let height = row_height(self.model, row);
                if height <= 0.0 {
                    continue;
                }
                self.current_row = Some(RowStrip { row, height });
                self.col_iter = self.pane.cols.clone();
                self.col_left = self.pane.origin.x;
            }

            if let Some(row_strip) = self.current_row.clone() {
                let Some(col) = self.col_iter.next() else {
                    self.row_top += row_strip.height;
                    self.current_row = None;
                    continue;
                };

                let width = col_width(self.model, col);
                if width <= 0.0 {
                    continue;
                }
                if self.col_left >= self.canvas.w {
                    self.row_top += row_strip.height;
                    self.current_row = None;
                    continue;
                }

                let rect = PixelRect {
                    top_left: Point {
                        x: self.col_left,
                        y: self.row_top,
                    },
                    width,
                    height: row_strip.height,
                };
                self.col_left += width;

                return Some(CellSlot {
                    addr: CellAddress {
                        sheet: self.sheet,
                        row: row_strip.row,
                        column: col,
                    },
                    rect,
                });
            }
        }
    }
}

/// Iterator decorator over `PaneCells`
/// queries the model.
pub(crate) struct CellPaintsIter<'a> {
    slots: PaneCells<'a>,
    renderer: &'a CanvasRenderer,
    model: &'a dyn CanvasModel,
}

impl<'a> CellPaintsIter<'a> {
    pub(crate) fn new(
        renderer: &'a CanvasRenderer,
        model: &'a dyn CanvasModel,
        pane: &'a PaneRegion,
    ) -> Self {
        Self {
            slots: pane.cells(model, renderer.canvas_size()),
            renderer,
            model,
        }
    }
}

impl<'a> Iterator for CellPaintsIter<'a> {
    type Item = CellPaint;

    fn next(&mut self) -> Option<CellPaint> {
        loop {
            let slot = self.slots.next()?;

            let Ok(own_style) =
                self.model
                    .get_cell_style(slot.addr.sheet, slot.addr.row, slot.addr.column)
            else {
                // Style fetch failed - skip this cell entirely (matches
                // today's `render_cell_style` early-return at cells.rs).
                continue;
            };

            let paint = CellPaint::resolve_cell_paint(self.renderer, slot, own_style);

            if let Some(p) = paint {
                return Some(p);
            }
        }
    }
}
