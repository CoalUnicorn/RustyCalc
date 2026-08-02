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
use self::text::TextPaint;
use crate::CellContentQuery;
use crate::chrome::{Chrome, PaneRegion};
use crate::orchestrator::PaneVerdict;
use crate::painter::Painter;
use crate::pending_work::RowSpan;
use crate::renderer::PaneExecution;
use crate::renderer::RendererCore;
use crate::renderer::cf_types::CfDecorationPaint;
use crate::renderer::prepared::{PaneCacheCommit, PreparedPane};
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
    ///
    /// Returns `true` when a transient bridge failure held this pane's prior
    /// pixels instead of painting (see the `reuses_slots` preflight below);
    /// `false` on every other exit, including the empty-pane early return.
    pub fn render_pane(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        frame: &Chrome,
    ) -> bool {
        match self.execute_pane(model, pane, frame) {
            PaneExecution::Held => true,
            PaneExecution::Untouched => false,
            PaneExecution::Committed(commit) => {
                self.install_pane_cache_commit(commit);
                false
            }
        }
    }

    pub(super) fn execute_pane(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        frame: &Chrome,
    ) -> PaneExecution {
        let Some(range) = pane.range(frame) else {
            // Pane became empty (e.g. freeze removed on this axis). Forget the
            // cached range so a future re-grow refetches. The painted tree
            // needs no explicit reset: a later re-grow builds a scratch tree
            // for a real range, and range-in-digest means it can't collide
            // with whatever stale tree sits in `painted`. Routed through the
            // one cache-commit entry point like every other outcome, rather
            // than mutated inline here.
            return PaneExecution::Committed(PaneCacheCommit::Empty { pane });
        };

        // A `Blitted` frame's fallback pane (`MissingCache`/`IncompatibleRange`)
        // is prepared directly by `RendererCore::prepare_blit`, which calls
        // this exact `prepare_full_pane` too — one fetch, no second `render_pane`
        // round-trip for the same cells. This entry point never adopts a
        // staged fetch of its own; it always fetches.
        let prepared = match self.prepare_full_pane(model, pane, range, frame) {
            Some(prepared) => prepared,
            None => {
                // The fetch reported a bridge failure — preparation touched
                // only its own renderer-lifetime scratch (see
                // `prepare_full_pane`'s doc), so the committed pane cache and
                // painted tree are exactly as the last successful paint left
                // them; a run of consecutive failures leaves them untouched
                // too.
                self.trace_pane(pane, PaneVerdict::Held);
                return PaneExecution::Held;
            }
        };

        let verdict = match &prepared {
            PreparedPane::Full { repaint, .. } => PaneVerdict::from(&repaint.plan),
            PreparedPane::Empty { .. }
            | PreparedPane::Damage { .. }
            | PreparedPane::Blit { .. } => {
                unreachable!("render_pane only ever prepares PreparedPane::Full")
            }
        };
        self.trace_pane(pane, verdict);

        PaneExecution::Committed(self.execute_full_pane(frame, prepared))
    }

    /// Shared paint tail for every prepared-execution method in
    /// `renderer::prepared` (`execute_full_pane`, `execute_damage_pane`,
    /// `execute_blit_pane`): the five deferred passes (bg -> CF decoration ->
    /// grid borders -> explicit borders -> text) over `cells`, reading from
    /// the four bulk buffers *by mutable borrow* rather than by value, so a
    /// multi-span/multi-strip caller can invoke this once per span against
    /// the SAME four buffers without re-taking them from `pane_buf` or
    /// parking them back in between — ownership and the take/park lifecycle
    /// live entirely with the caller. Pass order is load-bearing — see the
    /// doc on `render_pane` for why bg precedes borders precedes text.
    /// `pub(super)` so `renderer::prepared`'s execute methods can call it
    /// directly.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint_cells_pass(
        &self,
        cells: PaneCells,
        range: RCRange,
        theme: &CanvasTheme,
        pane_styles: &mut [Fetched<CellStyle>],
        pane_values: &mut [Fetched<String>],
        pane_cell_types: &mut [Fetched<CellKind>],
        pane_decorations: &mut [Fetched<CellDecoration>],
    ) {
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
            let idx = ((p.row - range.r1) * cols_w + (p.col - range.c1)) as usize;
            let Some(text) = pane_values.get_mut(idx).and_then(Fetched::take_value) else {
                continue;
            };
            let cell_type = pane_cell_types
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

    /// Damage-frame entry for one pane: repaint only the full-width row
    /// bands in `spans`. Every span that intersects this pane's range is
    /// fetched (`renderer::prepared::RendererCore::prepare_damage_pane`)
    /// BEFORE any of them splice into the cached buffers or paint — a
    /// later span's bridge failure can never leave an earlier, healthy
    /// span partially committed. Kept rows keep their pixels; a successful
    /// batch splices into the pane buffers but leaves the painted
    /// fingerprint tree alone (see `prepare_damage_pane`'s doc).
    ///
    /// Returns `true` when this pane's work was held rather than committed:
    /// either the range-mismatch demotion forwards `render_pane`'s own
    /// verdict, or any span's strip fetch failed.
    pub fn render_pane_damage(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        pane: PaneRegion,
        spans: &[RowSpan],
    ) -> bool {
        match self.execute_pane_damage(model, frame, pane, spans) {
            PaneExecution::Held => true,
            PaneExecution::Untouched => false,
            PaneExecution::Committed(commit) => {
                self.install_pane_cache_commit(commit);
                false
            }
        }
    }

    pub(super) fn execute_pane_damage(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        pane: PaneRegion,
        spans: &[RowSpan],
    ) -> PaneExecution {
        let pane_buf = self.pane_cache.pane(pane);
        let Some(range) = pane.range(frame) else {
            // Same empty-pane rationale as `render_pane`'s early return.
            return PaneExecution::Committed(PaneCacheCommit::Empty { pane });
        };
        // `splice_strip_into` indexes the cached pane buffers; they are
        // only aligned when the cached range matches this frame's. A
        // mismatch (e.g. partial post-blit buffers) demotes the pane to
        // the full walk instead of splicing at wrong indices.
        if pane_buf.range.get() != Some(range) {
            return self.execute_pane(model, pane, frame);
        }

        match self.prepare_damage_pane(model, frame, pane, range, spans) {
            None => {
                // Held: `prepare_damage_pane` already traced it and touched
                // nothing persistent.
                PaneExecution::Held
            }
            Some(PreparedPane::Damage { strips, .. }) if strips.is_empty() => {
                // No span in `spans` intersected this pane's range at all.
                PaneExecution::Untouched
            }
            Some(prepared) => PaneExecution::Committed(self.execute_damage_pane(frame, prepared)),
        }
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
