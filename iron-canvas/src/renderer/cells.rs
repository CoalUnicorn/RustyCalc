//! Cell painting - pure pixel pusher.
//!
//! `render_pane` iterates the resolved-paint stream from `CellPaintsIter`
//! and hands each `CellPaint` to `paint_cell`. Nothing in this file talks
//! to the model: bg, borders, and text are pre-resolved upstream.

use std::ops::RangeInclusive;

use crate::model::CssColor;
use crate::renderer::pane::PaneRegion;
use crate::theme::CanvasTheme;
use crate::types::text_paint::TextPaint;
use crate::{col_width, row_height, CanvasModel, CanvasSize, Point};

use super::super::geometry::{BorderEdge, FrameContext, PixelRect};
use super::super::model::{CellAddress, RCRange};
// use super::super::types::{resolve_cell_paint, resolve_text_paint, BorderPaint, CellPaint};
use super::CanvasRenderer;
use crate::renderer::{MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

use ironcalc_base::types::{BorderItem, BorderStyle, Style};

impl CanvasRenderer {
    /// Walk one frozen-pane quadrant. Streams bg+borders directly from the
    /// paint iterator and parks per-cell text inputs in `text_slots` for a
    /// second pass. Text paints last so overflow is never clipped by a
    /// neighbour's background.
    pub(super) fn render_pane(&self, model: &dyn CanvasModel, pane: PaneRegion) {
        let mut text_slots = self.text_slots.take();
        text_slots.clear();
        for p in self.paints_in(model, &pane) {
            self.paint_bg(&p);
            self.paint_borders(&p);
            text_slots.push(TextSlot {
                addr: p.addr,
                rect: p.rect,
                style: p.style,
            });
        }
        for t in &text_slots {
            if let Some(tp) = TextPaint::resolve(self, model, t.addr, t.rect, &t.style) {
                self.paint_text(&tp);
            }
        }
        self.text_slots.set(text_slots);
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
        self.paint_borders(p);
    }

    /// Stroke all four edges of a cell. Grid color is the base coat on every
    /// edge; any explicit `BorderItem` from the cell's style paints over it.
    pub(super) fn paint_borders(&self, p: &CellPaint) {
        let theme = self.theme();
        let b = &p.style.border;
        let edges = [
            (BorderEdge::Left, &b.left),
            (BorderEdge::Top, &b.top),
            (BorderEdge::Right, &b.right),
            (BorderEdge::Bottom, &b.bottom),
        ];
        for (edge, item) in edges {
            self.paint_border(edge, p.rect, &BorderPaint::grid_line(theme));
            if let Some(item) = item {
                self.paint_border(edge, p.rect, &BorderPaint::resolve(item, theme));
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
        let Some(rect) = self.range_pixel_bounds(frame, range) else {
            return;
        };
        let Ok(own_style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };
        let Some(paint) = CellPaint::resolve_cell_paint(
            self,
            //show_grid,
            CellSlot { addr, rect },
            &own_style,
        ) else {
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
    pub bg: String,
    pub style: Style,
}

/// Text-pass input parked during the streaming bg/border walk so the second
/// pass can paint text on top of every neighbour's already-laid background.
pub(super) struct TextSlot {
    pub(super) addr: CellAddress,
    pub(super) rect: PixelRect,
    pub(super) style: Style,
}

impl CellPaint {
    /// Resolve one cell into renderer-ready `CellPaint`. Combines the
    /// background fill, four border edges (with neighbour fallback for
    /// left/top), and optional text layout. Infallible on the style-fetch
    /// path: neighbour styles are passed in by the iterator.
    pub fn resolve_cell_paint(
        renderer: &CanvasRenderer,
        //show_grid: bool,
        slot: CellSlot,
        own_style: &Style,
    ) -> Option<CellPaint> {
        if slot.rect.width <= 0.0 || slot.rect.height <= 0.0 {
            return None;
        }

        let theme = renderer.theme();
        let bg = own_style
            .fill
            .fg_color
            .as_deref()
            .unwrap_or(theme.cell_bg)
            .to_owned();

        Some(CellPaint {
            addr: slot.addr,
            rect: slot.rect,
            bg,
            style: own_style.clone(),
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
    /// Thin grid-color stroke — the base coat painted on every edge before
    /// any explicit border style. Zero allocation: theme color is `&'static str`.
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
                CssColor::new(item.color.as_deref().unwrap_or(theme.grid_color)).0,
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

/// Iterator decorator over `PaneCells` that yields fully resolved
/// `CellPaint` per visible cell. Threads previous-row/column styles for
/// inner-edge neighbour fallback so the renderer only paints, never
/// queries the model.
pub(crate) struct CellPaintsIter<'a> {
    slots: PaneCells<'a>,
    renderer: &'a CanvasRenderer,
    model: &'a dyn CanvasModel,
}

impl<'a> CellPaintsIter<'a> {
    /// Build the resolved-paint stream for one pane. `show_grid` is read
    /// **once** here from the model and cached - replaces today's
    /// per-cell `get_show_grid_lines` call inside `render_cell_style`.
    pub(crate) fn new(
        renderer: &'a CanvasRenderer,
        model: &'a dyn CanvasModel,
        pane: &'a PaneRegion,
    ) -> Self {
        //let sheet = model.get_selected_sheet();
        //let show_grid = false; //model.get_show_grid_lines(sheet).unwrap_or(true);
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

            let paint = CellPaint::resolve_cell_paint(
                self.renderer,
                //self.show_grid,
                slot,
                &own_style,
            );

            if let Some(p) = paint {
                return Some(p);
            }
        }
    }
}
