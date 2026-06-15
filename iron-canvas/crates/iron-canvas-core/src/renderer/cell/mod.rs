//! Cell paint module.
//!
//! Three layers, mirroring the responsibilities of the per-cell pipeline:
//!
//! - [`paint`] — `CellPaint` (resolved per-cell paint), `PaneCells` (the
//!   per-quadrant walk that yields it), and the bg / single-cell entry
//!   points (`paint_bg`, `repaint_active_cell`).
//! - [`borders`] — `ResolvedBorders`, `BorderPaint`, and the grid /
//!   explicit / single-cell border passes.
//! - This module — `render_pane` (the five-pass walk over one quadrant)
//!   and `paint_cell` (single-cell composer).
//!
//! Pass order in `render_pane` is load-bearing: bg -> CF decoration ->
//! grid borders -> explicit borders -> text. See the doc on `render_pane`
//! for why.

pub mod borders;
pub mod fingerprint;
pub mod paint;
pub mod text;

pub use paint::{CellPaint, PaneCells};

use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::fetched::Fetched;

use self::borders::BorderPaint;
use self::fingerprint::compute_pane_fingerprint;
use self::text::TextPaint;
use crate::CellContentQuery;
use crate::chrome::{Chrome, PaneRegion};
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::blit_work::BlitPaneWork;
use crate::renderer::cf_types::CfDecorationPaint;
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;

impl<P: Painter> RendererCore<P> {
    /// Walk one frozen-pane quadrant in five deferred passes:
    /// bg -> CF decoration -> grid borders -> explicit borders -> text.
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
    pub fn render_pane(&self, model: &dyn CellContentQuery, pane: PaneRegion, frame: &Chrome) {
        let pane_idx = pane as usize;
        let pane_buf = self.pane_cache.pane(pane);

        let Some(range) = pane.range(frame) else {
            // Pane became empty (e.g. freeze removed on this axis). Forget
            // the cached range so a future re-grow doesn't false-match.
            pane_buf.range.set(None);
            return;
        };

        let theme = &frame.theme;

        // Bulk-fetch styles + formatted values for the whole rectangular
        // range. UserModel default impls loop the per-cell accessors (no perf
        // change); JsBackedModel will override (W5) and collapse each to one
        // JS call per pane.
        let mut pane_styles = pane_buf.styles.take();
        model.get_cell_styles_in(frame.sheet, range, &mut pane_styles);
        let mut pane_values = pane_buf.values.take();
        model.get_formatted_cell_values_in(frame.sheet, range, &mut pane_values);
        let mut pane_cell_types = pane_buf.cell_types.take();
        model.get_cell_types_in(frame.sheet, range, &mut pane_cell_types);
        let mut pane_decorations = pane_buf.decorations.take();
        model.get_cell_decorations_in(frame.sheet, range, &mut pane_decorations);

        // Fingerprint paint-skip: same content as the previous frame
        // ⇒ canvas pixels are still correct, skip the five-pass walk. Bulk
        // fetch above is unconditional now — content changes that don't
        // raise CONTENT (e.g. recalc triggered by an upstream edit a
        // caller forgot to mark) are detected here via fingerprint
        // mismatch, not assumed away by a geometric early-exit.
        let new_fp = compute_pane_fingerprint(&pane_styles, &pane_values, &pane_cell_types, range);
        let mut fps = frame.pane_fingerprints.get();
        fps[pane_idx] = new_fp;
        frame.pane_fingerprints.set(fps);

        if frame.kind.reuses_slots() {
            if new_fp == frame.prev_pane_fingerprints[pane_idx] {
                pane_buf.styles.set(pane_styles);
                pane_buf.values.set(pane_values);
                pane_buf.cell_types.set(pane_cell_types);
                // Set decorations back too: a later blit `try_shift` rotates
                // this buffer against the cached `Some(range)`; an empty vec
                // would misalign indices and yield wrong decorations.
                pane_buf.decorations.set(pane_decorations);
                pane_buf.range.set(Some(range));
                return;
            }
            // Content changed on a reused-canvas frame: clear the pane bg
            // so cells whose data just disappeared don't leave stale pixels.
            if let Some(pane_rect) = frame.range_rect(range) {
                self.painter
                    .rect_fill(pane_rect, PaintColor::from_theme_str(&theme.cell_bg));
            }
        }

        self.paint_pane_cells(
            PaneCells::new(&pane, frame),
            pane,
            range,
            theme,
            pane_styles,
            pane_values,
            pane_cell_types,
            pane_decorations,
        );
    }

    /// Shared paint tail for both `render_pane` and `render_pane_strip`:
    /// the five deferred passes (bg -> CF decoration -> grid borders ->
    /// explicit borders -> text) over `cells`, reading from the four bulk
    /// buffers and parking them back onto `pane`'s cache. The two callers
    /// differ only in which `PaneCells` they hand in — the full quadrant
    /// (`new`) or the revealed strip (`for_strip`); the pass machinery is
    /// identical, so it lives here once. Pass order is load-bearing — see
    /// the doc on `render_pane` for why bg precedes borders precedes text.
    #[allow(clippy::too_many_arguments)]
    fn paint_pane_cells(
        &self,
        cells: PaneCells,
        pane: PaneRegion,
        range: RCRange,
        theme: &CanvasTheme,
        mut pane_styles: Vec<Fetched<CellStyle>>,
        mut pane_values: Vec<Fetched<String>>,
        mut pane_cell_types: Vec<Fetched<CellKind>>,
        mut pane_decorations: Vec<Option<CellDecoration>>,
    ) {
        let pane_buf = self.pane_cache.pane(pane);
        let cols_w = range.c2 - range.c1 + 1;

        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();
        for slot in cells {
            let idx = ((slot.row - range.r1) * cols_w + (slot.col - range.c1)) as usize;
            let Some(own_style) = pane_styles.get_mut(idx).and_then(Fetched::take_value) else {
                continue;
            };
            // `own_style` already holds the dxf-merged CellStyle (the bridge folds
            // the CF overlay in get_cell_styles_in). The decoration rides the
            // same bulk buffer, indexed alongside styles/values/types.
            let cf_decoration = pane_decorations
                .get_mut(idx)
                .and_then(Option::take)
                .map(|deco| CfDecorationPaint::from_cell_decoration(&deco));
            let Some(mut p) =
                CellPaint::resolve_cell_paint(slot, own_style, theme, &self.color_intern)
            else {
                continue;
            };
            p.cf_decoration = cf_decoration;
            self.paint_bg(&p, theme);
            slots.push(p);
        }
        pane_buf.styles.set(pane_styles);
        pane_buf.decorations.set(pane_decorations);
        // CF decoration pass: data bars / icons / ratings overlay the cell
        // fill, below grid/explicit borders so the bar doesn't obscure border
        // strokes. Each decoration resolves into `Painter` primitives at the
        // renderer (`CfDecorationPaint::paint`), so no backend carries a
        // CF-specific method — every surface replays the same rect/path ops.
        for p in &slots {
            if let Some(ref deco) = p.cf_decoration {
                deco.paint(&*self.painter, p.rect);
            }
        }
        // Grid-line BorderPaint is theme-only, identical for every cell in the
        // pass. Build once here instead of per slot inside `paint_borders_grid`
        // (B-3) — on host-page themes that was one `theme.grid_color` String
        // clone per cell.
        let grid = BorderPaint::grid_line(theme);
        for p in &slots {
            self.paint_borders_grid(p, &grid);
        }
        for p in &slots {
            self.paint_borders_explicit(p);
        }

        let mut text_lines = self.frame_cache.text_lines.take();
        for p in &slots {
            let idx = ((p.row - range.r1) * cols_w + (p.col - range.c1)) as usize;
            let Some(text) = pane_values.get_mut(idx).and_then(Fetched::take_value) else {
                continue;
            };
            let cell_type = pane_cell_types
                .get_mut(idx)
                .and_then(Fetched::take_value)
                .unwrap_or(CellKind::Text);
            if let Some(tp) = TextPaint::resolve_into(
                self,
                p.rect,
                &p.style,
                text,
                cell_type,
                &mut text_lines,
            ) {
                self.paint_text(&tp, theme, &text_lines);
            }
        }
        pane_buf.values.set(pane_values);
        pane_buf.cell_types.set(pane_cell_types);
        pane_buf.range.set(Some(range));
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

    /// Blit-frame entry: paint only the revealed strip. The cache rotation
    /// (`try_shift`) and the strip/axis/clip computation already happened in
    /// `render_grid_blit` — this consumes the precomputed [`BlitPaneWork`] and
    /// fetches + paints the strip cells; kept-band cells keep their blitted
    /// pixels and are skipped via their `None` slots.
    pub fn render_pane_blit(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        work: &BlitPaneWork,
    ) {
        let pane = work.pane;
        let pane_idx = pane as usize;
        let pane_buf = self.pane_cache.pane(pane);
        let Some(range) = pane.range(frame) else {
            pane_buf.range.set(None);
            return;
        };
        self.render_pane_strip(model, pane, range, pane_idx, frame, work.strip_range);
    }

    /// Stage 3.3 strip path: kept-band pixels were preserved by the
    /// painter blit; the freshly-revealed strip subrange (`strip`, precomputed
    /// in `render_grid_blit`) is fetched from the model and painted, kept-band
    /// cells are skipped via their `None` slots. Sets `pane_fingerprints[idx]`
    /// to 0 — the partial buffer can't produce a content fingerprint for next
    /// frame's Stage 1 compare, so next frame falls through to a full
    /// bulk-fetch path.
    fn render_pane_strip(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        range: RCRange,
        pane_idx: usize,
        frame: &Chrome,
        strip: RCRange,
    ) {
        let theme = &frame.theme;
        let pane_buf = self.pane_cache.pane(pane);

        // Strip-fetch scratch reused from `FrameCache` (take/set rhythm),
        // not `Vec::new()` per frame — `splice_strip_into` drains these into
        // the pane buffers, leaving warm capacity to park back below. The
        // `*_in` defaults `clear()` before filling, so prior contents are
        // harmless.
        let mut strip_styles = self.frame_cache.strip_styles.take();
        let mut strip_values = self.frame_cache.strip_values.take();
        let mut strip_cell_types = self.frame_cache.strip_cell_types.take();
        let mut strip_decorations = self.frame_cache.strip_decorations.take();
        model.get_cell_styles_in(frame.sheet, strip, &mut strip_styles);
        model.get_formatted_cell_values_in(frame.sheet, strip, &mut strip_values);
        model.get_cell_types_in(frame.sheet, strip, &mut strip_cell_types);
        model.get_cell_decorations_in(frame.sheet, strip, &mut strip_decorations);

        let mut pane_styles = pane_buf.styles.take();
        let mut pane_values = pane_buf.values.take();
        let mut pane_cell_types = pane_buf.cell_types.take();
        let mut pane_decorations = pane_buf.decorations.take();
        splice_strip_into(&mut pane_styles, &mut strip_styles, range, strip);
        splice_strip_into(&mut pane_values, &mut strip_values, range, strip);
        splice_strip_into(&mut pane_cell_types, &mut strip_cell_types, range, strip);
        splice_strip_into(&mut pane_decorations, &mut strip_decorations, range, strip);
        self.frame_cache.strip_styles.set(strip_styles);
        self.frame_cache.strip_values.set(strip_values);
        self.frame_cache.strip_cell_types.set(strip_cell_types);
        self.frame_cache.strip_decorations.set(strip_decorations);

        if let Some(strip_rect) = frame.range_rect(strip) {
            self.painter
                .rect_fill(strip_rect, PaintColor::from_theme_str(&theme.cell_bg));
        }

        // Walk strip cells only. `apply_blit_shift` rotated the kept-band
        // entries (still `Some(...)`) into their new pane indices, so a
        // full-pane walk would re-`take` and re-paint the kept band on top
        // of pixels the painter blit already placed — wasting the entire
        // win. `PaneCells::for_strip` narrows the slot slices up front.
        self.paint_pane_cells(
            PaneCells::for_strip(&pane, frame, strip),
            pane,
            range,
            theme,
            pane_styles,
            pane_values,
            pane_cell_types,
            pane_decorations,
        );

        let mut fps = frame.pane_fingerprints.get();
        fps[pane_idx] = 0;
        frame.pane_fingerprints.set(fps);
    }
}

/// Move the freshly-fetched strip cells into `pane_buf` at the indices
/// corresponding to their `(row, col)` within the new pane range. Drains
/// `strip_buf` via `mem::swap` (no `Default`/`Clone` bound, so it serves both
/// `Fetched<T>` and `Option<T>` buffers): the pane slot's stale value lands
/// in the strip scratch, which the caller's next `*_in` fetch `clear()`s.
fn splice_strip_into<E>(
    pane_buf: &mut [E],
    strip_buf: &mut [E],
    pane_range: RCRange,
    strip_range: RCRange,
) {
    let pane_cols = (pane_range.c2 - pane_range.c1 + 1) as usize;
    let strip_cols = (strip_range.c2 - strip_range.c1 + 1) as usize;
    let strip_rows = (strip_range.r2 - strip_range.r1 + 1) as usize;
    debug_assert_eq!(strip_buf.len(), strip_rows * strip_cols);
    let row_offset = (strip_range.r1 - pane_range.r1) as usize;
    let col_offset = (strip_range.c1 - pane_range.c1) as usize;
    let pane_rows = pane_buf
        .chunks_exact_mut(pane_cols)
        .skip(row_offset)
        .take(strip_rows);
    let strip_rows_iter = strip_buf.chunks_exact_mut(strip_cols);
    for (pane_row, strip_row) in pane_rows.zip(strip_rows_iter) {
        let dst = &mut pane_row[col_offset..col_offset + strip_cols];
        for (d, s) in dst.iter_mut().zip(strip_row.iter_mut()) {
            std::mem::swap(d, s);
        }
    }
}
