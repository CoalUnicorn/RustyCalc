//! Cell paint module.
//!
//! Three layers, mirroring the responsibilities of the per-cell pipeline:
//!
//! - [`paint`] — `CellPaint` (resolved per-cell paint), `PaneCells` (the
//!   per-quadrant walk that yields it), and the bg / single-cell entry
//!   points (`paint_bg`, `repaint_active_cell`).
//! - [`borders`] — `ResolvedBorders`, `BorderPaint`, and the grid /
//!   explicit / single-cell border passes.
//! - This module — the five-pass walk over one grid segment
//!   and `paint_cell` (single-cell composer).
//!
//! Pass order is load-bearing: bg -> CF decoration -> grid borders ->
//! explicit borders -> text. See the doc on `paint_cells_pass`
//! for why.

pub mod borders;
pub mod fingerprint;
pub mod paint;
pub mod repaint;
pub mod text;

pub use paint::{CellPaint, PaneCells};

use crate::style::CellKind;
use crate::types::fetched::Fetched;

use self::borders::BorderPaint;
use self::text::TextPaint;
use crate::painter::Painter;
use crate::renderer::RendererCore;
use crate::renderer::cf_types::CfDecorationPaint;
use crate::renderer::prepared::FetchedCellsMut;
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;

impl<P: Painter> RendererCore<P> {
    /// Shared paint tail for every prepared-execution method in
    /// `renderer::prepared` (`execute_full_pane`, `execute_damage_pane`,
    /// `execute_blit_pane`): the five deferred passes (bg -> CF decoration ->
    /// grid borders -> explicit borders -> text) over `cells`, reading the
    /// fetched channels *by mutable borrow* ([`FetchedCellsMut`]) rather
    /// than by value, so a multi-span/multi-strip caller can invoke this
    /// once per span against the SAME owned [`FetchedCells`] bundle without
    /// re-taking it from `pane_buf` or parking it back in between —
    /// ownership and the take/park lifecycle live entirely with the caller.
    /// `index_range` is the address domain of the dense fetched channels;
    /// it may be larger than the `cells` walk during a partial repaint.
    /// Pass order is load-bearing — see the module doc for why bg
    /// precedes borders precedes text. `pub(super)` so `renderer::prepared`'s
    /// execute methods can call it directly.
    ///
    /// [`FetchedCells`]: crate::renderer::prepared::FetchedCells
    pub(super) fn paint_cells_pass(
        &self,
        cells: PaneCells,
        index_range: RCRange,
        theme: &CanvasTheme,
        fetched: FetchedCellsMut<'_>,
    ) {
        let cols_w = index_range.c2 - index_range.c1 + 1;

        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();
        for slot in cells {
            let idx = ((slot.row - index_range.r1) * cols_w + (slot.col - index_range.c1)) as usize;
            let Some(own_style) = fetched.styles.get_mut(idx).and_then(Fetched::take_value) else {
                continue;
            };
            // `own_style` already holds the dxf-merged CellStyle (the bridge folds
            // the CF overlay in get_cell_styles_in). The decoration rides the
            // same bulk buffer, indexed alongside styles/values/types.
            let cf_decoration = fetched
                .decorations
                .get_mut(idx)
                .and_then(Fetched::take_value)
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
            let idx = ((p.row - index_range.r1) * cols_w + (p.col - index_range.c1)) as usize;
            let Some(text) = fetched.values.get_mut(idx).and_then(Fetched::take_value) else {
                continue;
            };
            let cell_type = fetched
                .cell_types
                .get_mut(idx)
                .and_then(Fetched::take_value)
                .unwrap_or(CellKind::Text);
            if let Some(tp) =
                TextPaint::resolve_into(self, p.rect, &p.style, text, cell_type, &mut text_lines)
            {
                self.paint_text(&tp, theme, &text_lines);
            }
        }
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

/// `pub(super)` so `renderer::prepared::FetchedCells::has_bridge_failure`
/// (checking all four channels as one bundle) can reuse this same
/// per-channel predicate instead of duplicating it.
pub(super) fn has_bridge_failure<T>(items: &[Fetched<T>]) -> bool {
    items.iter().any(Fetched::is_bridge_failed)
}

/// Move the freshly-fetched strip cells into `pane_buf` at the indices
/// corresponding to their `(row, col)` within the new pane range. Drains
/// `strip_buf` via `mem::swap` (no `Default`/`Clone` bound, so it serves both
/// `Fetched<T>` and `Option<T>` buffers): the pane slot's stale value lands
/// in the strip scratch, which the caller's next `*_in` fetch `clear()`s.
/// `pub(super)` so `renderer::prepared`'s multi-strip Damage execution can
/// reuse it (single-strip splice, called once per prepared strip).
pub(super) fn splice_strip_into<E>(
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
