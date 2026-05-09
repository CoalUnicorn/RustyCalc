//! Cell painting.
//!
//! `render_pane` walks one frozen-pane quadrant in four deferred sub-passes
//! over the same `CellPaint` slots: bg -> grid borders -> explicit borders ->
//! text. Styles are bulk-fetched once per pane via
//! `CanvasModel::get_cell_styles_in` into a dense row-major buffer; the bg
//! pass moves each `Style` out via `Option::take` and `CellPaint::resolve`
//! lifts it into renderer-ready paint. `BorderPaint::resolve` and
//! `TextPaint::resolve` run inside their respective sub-passes so border
//! and text work happens only on cells that reach them.

use std::borrow::Cow;
use std::rc::Rc;

use crate::geometry::frame::slot::{ColSlot, RowSlot};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::{BorderEdge, Point};
use crate::painter::{PaintColor, Painter};
use crate::renderer::cache::ColorIntern;
use crate::renderer::pane::PaneRegion;
use crate::renderer::text_paint::TextPaint;
use crate::theme::CanvasTheme;
use crate::CanvasModel;

use super::super::geometry::frame::FrameContext;
use super::super::types::coord::{CellAddress, RCRange};
use super::RendererCore;
use crate::geometry::constants::{MEDIUM_BORDER_WIDTH, STANDARD_BORDER_WIDTH, THICK_BORDER_WIDTH};

use ironcalc_base::types::{Border, BorderItem, BorderStyle, Style};

impl<P: Painter> RendererCore<P> {
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
    /// across all -> explicit across all -> A.right strokes last on the
    /// shared edge). Text remains the final pass so overflow is never
    /// clipped by a neighbour's bg.
    pub(super) fn render_pane(
        &self,
        model: &dyn CanvasModel,
        pane: PaneRegion,
        frame: &FrameContext,
    ) {
        let Some(range) = pane.range(frame) else {
            return;
        };

        let theme = &frame.theme;
        let cols_w = range.c2 - range.c1 + 1;

        // Bulk-fetch styles + formatted values for the whole rectangular
        // range. UserModel default impls loop the per-cell accessors (no perf
        // change); JsBackedModel will override (W5) and collapse each to one
        // JS call per pane.
        let mut pane_styles = self.frame_cache.pane_styles.take();
        model.get_cell_styles_in(frame.sheet, range, &mut pane_styles);
        let mut pane_values = self.frame_cache.pane_values.take();
        model.get_formatted_cell_values_in(frame.sheet, range, &mut pane_values);

        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();

        for slot in PaneCells::new(&pane, frame) {
            let idx = ((slot.addr.row - range.r1) * cols_w
                + (slot.addr.column - range.c1)) as usize;
            let Some(own_style) = pane_styles.get_mut(idx).and_then(Option::take) else {
                continue;
            };
            let Some(p) =
                CellPaint::resolve_cell_paint(slot, own_style, theme, &self.color_intern)
            else {
                continue;
            };
            self.paint_bg(&p, theme);
            slots.push(p);
        }
        self.frame_cache.pane_styles.set(pane_styles);
        for p in &slots {
            self.paint_borders_grid(p, theme);
        }
        for p in &slots {
            self.paint_borders_explicit(p);
        }

        let mut text_lines = self.frame_cache.text_lines.take();
        for p in &slots {
            let idx = ((p.addr.row - range.r1) * cols_w + (p.addr.column - range.c1)) as usize;
            let Some(text) = pane_values.get_mut(idx).and_then(Option::take) else {
                continue;
            };
            if let Some(tp) = TextPaint::resolve_into(
                self,
                model,
                p.addr,
                p.rect,
                theme,
                &p.style,
                text,
                &mut text_lines,
            ) {
                self.paint_text(&tp, &text_lines);
            }
        }
        self.frame_cache.pane_values.set(pane_values);
        self.frame_cache.text_slots.set(slots);
        self.frame_cache.text_lines.set(text_lines);
    }

    /// Fill a cell's background rectangle. Border pass is separate (batched).
    pub(super) fn paint_bg(&self, p: &CellPaint, theme: &CanvasTheme) {
        // Branch on the model's per-cell override: zero-alloc Static path for
        // the theme default, Borrowed for the colored case. Avoids feeding
        // every theme-default cell through the painter's allocating cache miss.
        let color = match p.style.fill.fg_color.as_deref() {
            Some(c) => PaintColor::Borrowed(c),
            None => PaintColor::from_theme_str(&theme.cell_bg),
        };
        self.painter.rect_fill(p.rect, color);
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead; the main pane pass uses `paint_borders`.
    pub(super) fn paint_cell(&self, p: &CellPaint, theme: &CanvasTheme) {
        self.paint_bg(p, theme);
        self.paint_borders(p, theme);
    }

    /// Grid-fallback strokes on left+top, gated by show-grid + fill-suppression.
    /// Reads `p.borders` (pre-resolved at iteration time) so an explicit
    /// border on left/top still suppresses the grid stroke without re-walking
    /// `style.border`.
    fn paint_borders_grid(&self, p: &CellPaint, theme: &CanvasTheme) {
        if !self.frame_cache.show_grid.get() {
            return;
        }
        if p.style.fill.fg_color.is_some() {
            return;
        }
        let grid = BorderPaint::grid_line(theme);
        if p.borders.left.is_none() {
            self.paint_border(BorderEdge::Left, p.rect, &grid);
        }
        if p.borders.top.is_none() {
            self.paint_border(BorderEdge::Top, p.rect, &grid);
        }
    }

    /// Stroke pre-resolved explicit borders on the cell's four edges. Pure
    /// pixel pushing — no `BorderPaint::resolve` calls inside the loop.
    /// Run across every slot AFTER the grid sub-pass so an explicit right on
    /// cell A wins over cell B's grid left at the shared pixel column.
    fn paint_borders_explicit(&self, p: &CellPaint) {
        if let Some(b) = &p.borders.left {
            self.paint_border(BorderEdge::Left, p.rect, b);
        }
        if let Some(b) = &p.borders.top {
            self.paint_border(BorderEdge::Top, p.rect, b);
        }
        if let Some(b) = &p.borders.right {
            self.paint_border(BorderEdge::Right, p.rect, b);
        }
        if let Some(b) = &p.borders.bottom {
            self.paint_border(BorderEdge::Bottom, p.rect, b);
        }
    }

    /// Single-cell border paint used by `repaint_active_cell` where there
    /// are no neighbour interactions to worry about. Composes the two
    /// sub-passes in their canonical order: grid fallback first, explicit
    /// over the top.
    pub(super) fn paint_borders(&self, p: &CellPaint, theme: &CanvasTheme) {
        self.paint_borders_grid(p, theme);
        self.paint_borders_explicit(p);
    }

    /// Stroke one resolved border. `Double`-style borders render as two
    /// parallel strokes offset ±1px on the cross-axis.
    fn paint_border(&self, edge: BorderEdge, rect: PixelRect, b: &BorderPaint) {
        let line = edge.line(rect);
        let offsets: &[i32] = if b.stroke.double { &[-1, 1] } else { &[0] };
        // BorderColor::Static carries the theme grid color; the helper
        // routes Cow::Borrowed through the painter's ptr-eq fast path and
        // Cow::Owned through the content-eq cache. BorderColor::Owned is
        // the per-cell custom color via the interner.
        let color = match &b.color {
            BorderColor::Static(s) => PaintColor::from_theme_str(s),
            BorderColor::Owned(s) => PaintColor::Borrowed(s),
        };
        for &d in offsets {
            self.painter
                .stroke_line(line.offset_cross(d), color, f64::from(b.stroke.width_px));
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
        let Some(own_style) = model.get_cell_style(addr.sheet, addr.row, addr.column) else {
            return;
        };
        let theme = &frame.theme;
        let Some(paint) = CellPaint::resolve_cell_paint(
            CellSlot { addr, rect },
            own_style,
            theme,
            &self.color_intern,
        ) else {
            return;
        };
        self.paint_cell(&paint, theme);
        let mut text_lines = self.frame_cache.text_lines.take();
        if let Some(text) = model.get_formatted_cell_value(addr.sheet, addr.row, addr.column) {
            if let Some(t) = TextPaint::resolve_into(
                self,
                model,
                addr,
                rect,
                theme,
                &paint.style,
                text,
                &mut text_lines,
            ) {
                self.paint_text(&t, &text_lines);
            }
        }
        self.frame_cache.text_lines.set(text_lines);
    }
}

pub(crate) struct CellPaint {
    pub addr: CellAddress,
    pub rect: PixelRect,
    pub style: Style,
    /// Per-edge resolved border paints. Computed once at iteration time so
    /// the explicit-border sub-pass in `render_pane` is pure pixel pushing —
    /// no `BorderPaint::resolve` calls inside the paint loop.
    pub borders: ResolvedBorders,
}

/// Per-edge `BorderPaint` resolved from a cell's `Borders` style. `None` on
/// an edge means the cell carries no explicit border there — the grid
/// sub-pass will fill the left/top edges with the theme grid line.
pub(crate) struct ResolvedBorders {
    pub left: Option<BorderPaint>,
    pub top: Option<BorderPaint>,
    pub right: Option<BorderPaint>,
    pub bottom: Option<BorderPaint>,
}

impl ResolvedBorders {
    /// Resolve every `Some` `BorderItem` on `border` into a `BorderPaint`.
    /// Same shape as the old `paint_borders_explicit`: zero allocations on
    /// edges that fall back to the theme grid color (`BorderColor::Static`),
    /// `Rc::clone` per edge that carries an explicit color override (the
    /// renderer's `ColorIntern` absorbs the first-sighting alloc).
    fn resolve(border: &Border, theme: &CanvasTheme, intern: &ColorIntern) -> Self {
        Self {
            left: border
                .left
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            top: border
                .top
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            right: border
                .right
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
            bottom: border
                .bottom
                .as_ref()
                .map(|i| BorderPaint::resolve(i, theme, intern)),
        }
    }
}

impl CellPaint {
    /// Resolve one cell Style into renderer-ready `CellPaint`.
    /// Takes Style by value — the caller's owned copy is moved straight through
    /// to the paint, eliminating the per-cell clone on the hot pane walk. Borders
    /// are resolved here so the per-edge paint passes stay pure pixel pushers.
    pub fn resolve_cell_paint(
        slot: CellSlot,
        own_style: Style,
        theme: &CanvasTheme,
        intern: &ColorIntern,
    ) -> Option<CellPaint> {
        if slot.rect.width <= 0 || slot.rect.height <= 0 {
            return None;
        }
        let borders = ResolvedBorders::resolve(&own_style.border, theme, intern);
        Some(CellPaint {
            addr: slot.addr,
            rect: slot.rect,
            style: own_style,
            borders,
        })
    }
}

pub(crate) struct BorderStroke {
    pub width_px: i32,
    pub double: bool,
}

/// Color for a resolved border edge. `Static` carries the theme grid color as
/// `Cow<'static, str>` — built-in themes (`Cow::Borrowed`) ptr-eq through the
/// painter cache; host-page themes (`Cow::Owned`) clone the `String` once per
/// resolve and content-eq through the cache. `Owned` is the per-cell override
/// path, an interned `Rc<str>` from `ColorIntern` (`Rc::clone` after the first
/// sighting of each unique color).
pub(crate) enum BorderColor {
    Static(Cow<'static, str>),
    Owned(Rc<str>),
}

pub(crate) struct BorderPaint {
    pub color: BorderColor,
    pub stroke: BorderStroke,
}

impl BorderPaint {
    /// Thin grid-color stroke used as the right/bottom fallback when a cell
    /// has no explicit border on that edge. `Cow::clone` is a pointer copy
    /// for built-in themes (`Cow::Borrowed`) and a `String::clone` for
    /// host-page themes — the latter is the only per-cell allocation, and
    /// only on the new owned-theme path.
    fn grid_line(theme: &CanvasTheme) -> Self {
        Self {
            color: BorderColor::Static(theme.grid_color.clone()),
            stroke: BorderStroke {
                width_px: STANDARD_BORDER_WIDTH,
                double: false,
            },
        }
    }

    /// Resolve a `BorderItem` from the cell style into a renderer-ready paint.
    /// Color falls back to `theme.grid_color` when the item carries no explicit
    /// color — `Cow::clone` keeps the built-in path zero-alloc. The
    /// explicit-color branch goes through `ColorIntern` so the dynamic color
    /// is `Rc::clone`d on every frame after the first sighting.
    fn resolve(item: &BorderItem, theme: &CanvasTheme, intern: &ColorIntern) -> Self {
        let color = match item.color.as_deref() {
            None => BorderColor::Static(theme.grid_color.clone()),
            Some(c) => BorderColor::Owned(intern.get(c)),
        };
        Self {
            color,
            stroke: BorderStroke::from_border_style(&item.style),
        }
    }
}

impl BorderStroke {
    /// Map `BorderStyle` -> pixel width + double-line flag.
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

/// One cell yielded by a `PaneCells` walk: address + pixel rect at the
/// current scroll.
#[derive(Clone, Copy)]
pub(crate) struct CellSlot {
    pub addr: CellAddress,
    pub rect: PixelRect,
}

/// Stateful walk over the cells of a `PaneRegion`. Reads all per-cell
/// geometry (size, position, sheet, canvas extents) from the
/// `FrameContext` snapshot built once per tick — same source of truth as
/// `frame.cell_rect()` and the input layer's hit-test, so what's painted
/// can never disagree with what gets hit.
pub(crate) struct PaneCells<'a> {
    pub frame: &'a FrameContext,
    rows: std::slice::Iter<'a, RowSlot>,
    cols_template: &'a [ColSlot],
    cols: std::slice::Iter<'a, ColSlot>,
    current_row: Option<RowSlot>,
}

impl<'a> PaneCells<'a> {
    pub(crate) fn new(pane: &'a PaneRegion, frame: &'a FrameContext) -> Self {
        let cols_template = pane.cols(frame);
        Self {
            frame,
            rows: pane.rows(frame).iter(),
            cols_template,
            cols: cols_template.iter(),
            current_row: None,
        }
    }
}

impl<'a> Iterator for PaneCells<'a> {
    type Item = CellSlot;

    fn next(&mut self) -> Option<CellSlot> {
        loop {
            let row = match self.current_row {
                Some(r) => r,
                None => {
                    let r = *self.rows.next()?;
                    self.current_row = Some(r);
                    self.cols = self.cols_template.iter();
                    r
                }
            };
            let Some(col) = self.cols.next().copied() else {
                self.current_row = None;
                continue;
            };
            return Some(CellSlot {
                addr: CellAddress {
                    sheet: self.frame.sheet,
                    row: row.row,
                    column: col.col,
                },
                rect: PixelRect {
                    top_left: Point {
                        x: col.left,
                        y: row.top,
                    },
                    width: col.width,
                    height: row.height,
                },
            });
        }
    }
}

