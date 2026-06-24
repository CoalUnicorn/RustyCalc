//! Resolved cell paint + the `PaneCells` walk that yields it.
//!
//! `CellPaint` carries everything the per-cell paint passes need without
//! re-entering the model: pixel rect, owned `Style`, and per-edge
//! `ResolvedBorders`. The sheet is constant for one pane walk and lives on
//! `Chrome.sheet`, so per-cell records only carry `(row, col)`.
//! `paint_bg` is the bg-only painter used by the bulk pane walk;
//! `repaint_active_cell` is the single-cell entry point used by the
//! selection overlay to restore the active cell on top of the
//! semi-transparent selection fill.

use super::borders::ResolvedBorders;
use super::text::TextPaint;
use crate::CellContentQuery;
use crate::chrome::{Chrome, PaneRegion};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::slot::{ColSlot, RowSlot};
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::cache::ColorIntern;
use crate::renderer::cf_types::CfDecorationPaint;
use crate::style::{CellKind, CellStyle};
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;

pub struct CellPaint {
    pub row: i32,
    pub col: i32,
    pub rect: PixelRect,
    pub style: CellStyle,
    /// Per-edge resolved border paints. Computed once at iteration time so
    /// the explicit-border sub-pass in `render_pane` is pure pixel pushing —
    /// no `BorderPaint::resolve` calls inside the paint loop.
    pub borders: ResolvedBorders,
    /// Conditional-formatting decoration (data bar / icon / rating), if any
    /// CF rule with a decoration matches this cell. `None` for plain cells.
    /// CF *fill/font* overrides are not carried here — they ride the `style`
    /// field, which the render pass sources from `get_cell_styles_in` (the base
    /// `CellStyle` with the CF dxf overlay already folded in by the bridge).
    /// Resolved in the render loop (where the model is in scope), not in
    /// `resolve_cell_paint`.
    pub cf_decoration: Option<CfDecorationPaint>,
}

impl CellPaint {
    /// Resolve one cell `Style` into a renderer-ready `CellPaint`.
    /// Takes `Style` by value — the caller's owned copy moves straight
    /// through to the paint, eliminating the per-cell clone on the hot pane
    /// walk. Borders are resolved here so the per-edge paint passes stay
    /// pure pixel pushers.
    pub fn resolve_cell_paint(
        slot: CellSlot,
        own_style: CellStyle,
        theme: &CanvasTheme,
        intern: &ColorIntern,
    ) -> Option<CellPaint> {
        if slot.rect.width <= 0 || slot.rect.height <= 0 {
            return None;
        }
        let borders = ResolvedBorders::resolve(&own_style.border, theme, intern);
        Some(CellPaint {
            row: slot.row,
            col: slot.col,
            rect: slot.rect,
            style: own_style,
            borders,
            cf_decoration: None,
        })
    }
}

/// One cell yielded by a `PaneCells` walk: `(row, col)` + pixel rect at the
/// current scroll. Sheet is implicit — see `Chrome.sheet`.
#[derive(Clone, Copy)]
pub struct CellSlot {
    pub row: i32,
    pub col: i32,
    pub rect: PixelRect,
}

/// Stateful walk over the cells of a `PaneRegion`. Reads per-cell geometry
/// from the `Chrome` snapshot built once per tick — same source of truth as
/// `chrome.cell_rect()` and the input layer's hit-test, so what's painted
/// can never disagree with what gets hit.
pub struct PaneCells<'a> {
    rows: std::slice::Iter<'a, RowSlot>,
    cols_template: &'a [ColSlot],
    cols: std::slice::Iter<'a, ColSlot>,
    current_row: Option<RowSlot>,
}

impl<'a> PaneCells<'a> {
    pub fn new(pane: &'a PaneRegion, frame: &'a Chrome) -> Self {
        let cols_template = pane.cols(frame);
        Self {
            rows: pane.rows(frame).iter(),
            cols_template,
            cols: cols_template.iter(),
            current_row: None,
        }
    }

    /// Walk only the cells whose `(row, col)` lies inside `strip`. Used by
    /// the scroll-blit strip-fetch path: the painter blit preserved the
    /// kept-band pixels, so the five per-cell sub-passes must touch only
    /// the freshly-revealed strip — iterating the full pane would re-paint
    /// the kept band on top of the already-correct pixels.
    ///
    /// Slot vecs are sorted by `row` / `col`, so `partition_point` returns
    /// the half-open range of indices that intersect `strip` in O(log N).
    pub fn for_strip(pane: &'a PaneRegion, frame: &'a Chrome, strip: RCRange) -> Self {
        let rows_full = pane.rows(frame);
        let cols_full = pane.cols(frame);
        let r_start = rows_full.partition_point(|s| s.row < strip.r1);
        let r_end = rows_full.partition_point(|s| s.row <= strip.r2);
        let c_start = cols_full.partition_point(|s| s.col < strip.c1);
        let c_end = cols_full.partition_point(|s| s.col <= strip.c2);
        let cols_template = &cols_full[c_start..c_end];
        Self {
            rows: rows_full[r_start..r_end].iter(),
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
                row: row.row,
                col: col.col,
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

impl<P: Painter> RendererCore<P> {
    /// Fill a cell's background rectangle. Border pass is separate (batched).
    pub(super) fn paint_bg(&self, p: &CellPaint, theme: &CanvasTheme) {
        // Branch on the model's per-cell override: zero-alloc Static path for
        // the theme default, Borrowed for the colored case. Avoids feeding
        // every theme-default cell through the painter's allocating cache miss.
        let color = match p.style.fill_color.as_deref() {
            Some(c) => PaintColor::Borrowed(c),
            None => PaintColor::from_theme_str(&theme.cell_bg),
        };
        self.painter.rect_fill(p.rect, color);
    }

    /// Repaint one cell's full paint (bg + borders + text) at `(row, column)`
    /// on the active sheet. Used by the selection overlay to restore the
    /// active cell on top of the semi-transparent selection fill. Sheet is
    /// implicit — taken from `frame.sheet`.
    pub fn repaint_active_cell(
        &self,
        model: &dyn CellContentQuery,
        row: i32,
        column: i32,
        frame: &Chrome,
    ) {
        let sheet = frame.sheet;
        let range = RCRange::from_cell(row, column);
        let Some(rect) = frame.range_rect(range) else {
            return;
        };

        // Fetch every input *before* touching the painter. The overlay was
        // cleared at the top of the frame, so this method either fully repaints
        // the active cell or leaves the grid layer's prior pixels showing
        // through. A transient `BridgeFailed` on any field must take the second
        // path: painting an opaque background and then *skipping* the failed
        // text would hide the grid's correct content behind a blank box — the
        // A-3 flash. `Absent` is not a failure (a blank cell legitimately has
        // no text), so it does not abort. Native models never report
        // `BridgeFailed`, making this a no-op for every non-JS host.
        let style = model.get_cell_style(sheet, row, column);
        let value = model.get_formatted_cell_value(sheet, row, column);
        let cell_type = model.get_cell_type(sheet, row, column);
        if style.is_bridge_failed() || value.is_bridge_failed() || cell_type.is_bridge_failed() {
            return;
        }

        let Some(own_style) = style.value() else {
            return;
        };
        let theme = &frame.theme;
        let Some(paint) = CellPaint::resolve_cell_paint(
            CellSlot {
                row,
                col: column,
                rect,
            },
            own_style,
            theme,
            &self.color_intern,
        ) else {
            return;
        };
        self.paint_cell(&paint, theme);
        let mut text_lines = self.frame_cache.text_lines.take();
        if let Some(text) = value.value() {
            let cell_type = cell_type.unwrap_or(CellKind::Text);
            if let Some(t) =
                TextPaint::resolve_into(self, rect, &paint.style, text, cell_type, &mut text_lines)
            {
                self.paint_text(&t, theme, &text_lines);
            }
        }
        self.frame_cache.text_lines.set(text_lines);
    }
}
