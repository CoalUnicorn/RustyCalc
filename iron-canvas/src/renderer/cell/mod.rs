//! Cell paint module.
//!
//! Three layers, mirroring the responsibilities of the per-cell pipeline:
//!
//! - [`paint`] — `CellPaint` (resolved per-cell paint), `PaneCells` (the
//!   per-quadrant walk that yields it), and the bg / single-cell entry
//!   points (`paint_bg`, `repaint_active_cell`).
//! - [`borders`] — `ResolvedBorders`, `BorderPaint`, and the grid /
//!   explicit / single-cell border passes.
//! - This module — `render_pane` (the four-pass walk over one quadrant)
//!   and `paint_cell` (single-cell composer).
//!
//! Pass order in `render_pane` is load-bearing: bg -> grid borders ->
//! explicit borders -> text. See the doc on `render_pane` for why.

pub(crate) mod borders;
pub(crate) mod paint;
pub(crate) mod text;

pub(crate) use paint::{CellPaint, PaneCells};

use ironcalc_base::types::CellType;

use self::text::TextPaint;
use crate::chrome::{Chrome, PaneRegion};
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::theme::CanvasTheme;
use crate::CanvasModel;

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
    /// every boundary, matching Excel/Sheets.
    ///
    /// The grid sub-pass runs across all cells before the explicit-border
    /// sub-pass so an explicit `BorderItem::right` on cell A wins over
    /// cell B's grid left at the shared pixel column (paint order: grid
    /// across all -> explicit across all -> A.right strokes last on the
    /// shared edge). Text remains the final pass so overflow is never
    /// clipped by a neighbour's bg.
    pub(crate) fn render_pane(&self, model: &dyn CanvasModel, pane: PaneRegion, frame: &Chrome) {
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
        let mut pane_cell_types = self.frame_cache.pane_cell_types.take();
        model.get_cell_types_in(frame.sheet, range, &mut pane_cell_types);

        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();

        for slot in PaneCells::new(&pane, frame) {
            let idx = ((slot.row - range.r1) * cols_w + (slot.col - range.c1)) as usize;
            let Some(own_style) = pane_styles.get_mut(idx).and_then(Option::take) else {
                continue;
            };
            let Some(p) = CellPaint::resolve_cell_paint(slot, own_style, theme, &self.color_intern)
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
            let idx = ((p.row - range.r1) * cols_w + (p.col - range.c1)) as usize;
            let Some(text) = pane_values.get_mut(idx).and_then(Option::take) else {
                continue;
            };
            let cell_type = pane_cell_types
                .get_mut(idx)
                .and_then(Option::take)
                .unwrap_or(CellType::Text);
            if let Some(tp) = TextPaint::resolve_into(
                self,
                p.rect,
                theme,
                &p.style,
                text,
                cell_type,
                &mut text_lines,
            ) {
                self.paint_text(&tp, &text_lines);
            }
        }
        self.frame_cache.pane_values.set(pane_values);
        self.frame_cache.pane_cell_types.set(pane_cell_types);
        self.frame_cache.text_slots.set(slots);
        self.frame_cache.text_lines.set(text_lines);
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead.
    pub(super) fn paint_cell(&self, p: &CellPaint, theme: &CanvasTheme) {
        self.paint_bg(p, theme);
        self.paint_borders(p, theme);
    }
}
