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
pub(crate) mod fingerprint;
pub(crate) mod paint;
pub(crate) mod text;

pub(crate) use paint::{CellPaint, PaneCells};

use ironcalc_base::types::CellType;

use self::fingerprint::compute_pane_fingerprint;
use self::text::TextPaint;
use crate::chrome::{Chrome, PaneRegion};
use crate::geometry::prim::Axis;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;
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
        let pane_idx = pane as usize;
        let pane_buf = self.pane_cache.pane(pane);

        let Some(range) = pane.range(frame) else {
            // Pane became empty (e.g. freeze removed on this axis). Forget
            // the cached range so a future re-grow doesn't false-match.
            pane_buf.range.set(None);
            return;
        };

        let theme = &frame.theme;
        let cols_w = range.c2 - range.c1 + 1;

        // Stage 3.2 — geometric early exit. When the orchestrator carried
        // the prior frame over (`slots_reused`) and this pane's address
        // hasn't moved, the cached buffers + the painted pixels are still
        // valid by construction. Cross-frozen panes on a scroll-blit always
        // hit this branch; same-axis panes fall through to fetch + paint.
        //
        // We propagate the previous fingerprint forward so the next frame's
        // `prev_pane_fingerprints[idx]` still reflects what was painted at
        // this pane — otherwise the Stage 1 secondary skip would false-miss
        // on the next repaint.
        if frame.slots_reused && pane_buf.range.get() == Some(range) {
            let mut fps = frame.pane_fingerprints.get();
            fps[pane_idx] = frame.prev_pane_fingerprints[pane_idx];
            frame.pane_fingerprints.set(fps);
            return;
        }

        // Stage 3.3 — strip-fetch branch. `prepare_blit_cache` rotated
        // kept-band entries into their new indices (still `Some`) and left
        // the strip slots `None`; `pane_buf.range` stays at the pre-blit
        // value as the signal. Infer the single-axis shift, fetch only the
        // strip from the model, and let `render_pane_strip` confine the
        // four paint passes to the strip rect via `PaneCells::for_strip`.
        // Kept-band pixels are already correct (painter blit) and must NOT
        // be repainted on top.
        if frame.slots_reused {
            if let Some(prev_range) = pane_buf.range.get() {
                if let Some(axis) = infer_shift_axis(prev_range, range) {
                    self.render_pane_strip(model, pane, range, axis, prev_range, pane_idx, frame);
                    return;
                }
            }
        }

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

        // Stage 1 paint-skip: same content as the previous frame at the
        // same pane address ⇒ canvas pixels are still correct. Skip the
        // 4-pass walk entirely; the orchestrator's slot-reuse path also
        // suppressed the full canvas clear so the prior pixels survived.
        // This is the *secondary* skip; the primary range-equality skip
        // above avoided the fetch entirely when the address matched.
        let new_fp = compute_pane_fingerprint(&pane_styles, &pane_values, &pane_cell_types, range);
        let mut fps = frame.pane_fingerprints.get();
        fps[pane_idx] = new_fp;
        frame.pane_fingerprints.set(fps);

        if frame.slots_reused {
            if new_fp == frame.prev_pane_fingerprints[pane_idx] {
                pane_buf.styles.set(pane_styles);
                pane_buf.values.set(pane_values);
                pane_buf.cell_types.set(pane_cell_types);
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
        pane_buf.styles.set(pane_styles);
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

    /// Stage 3.3 strip path: kept-band pixels were preserved by the
    /// painter blit; the freshly-revealed strip subrange is fetched from
    /// the model and painted, kept-band cells are skipped via their
    /// `None` slots. Sets `pane_fingerprints[idx]` to 0 — the partial
    /// buffer can't produce a content fingerprint for next frame's Stage
    /// 1 compare, so next frame falls through to a full bulk-fetch path.
    #[allow(clippy::too_many_arguments)]
    fn render_pane_strip(
        &self,
        model: &dyn CanvasModel,
        pane: PaneRegion,
        range: RCRange,
        axis: Axis,
        prev_range: RCRange,
        pane_idx: usize,
        frame: &Chrome,
    ) {
        let theme = &frame.theme;
        let cols_w = range.c2 - range.c1 + 1;
        let pane_buf = self.pane_cache.pane(pane);

        let Some(strip) = compute_strip(prev_range, range, axis) else {
            pane_buf.range.set(Some(range));
            return;
        };
        #[cfg(target_arch = "wasm32")]
        {
            use crate::wasm::diag::console_log;
            let rect_dbg = frame
                .range_rect(strip)
                .map(|r| {
                    format!(
                        "({},{},{}x{})",
                        r.top_left.x, r.top_left.y, r.width, r.height
                    )
                })
                .unwrap_or_else(|| "<none>".to_string());
            let axis_s = match axis {
                Axis::Row => "Row",
                Axis::Column => "Col",
            };
            console_log(&format!(
                "[strip] axis={} pane_idx={} prev=r{}..{},c{}..{} new=r{}..{},c{}..{} strip=r{}..{},c{}..{} strip_rect={}",
                axis_s,
                pane_idx,
                prev_range.r1, prev_range.r2, prev_range.c1, prev_range.c2,
                range.r1, range.r2, range.c1, range.c2,
                strip.r1, strip.r2, strip.c1, strip.c2,
                rect_dbg,
            ));
        }

        let mut strip_styles = Vec::new();
        let mut strip_values = Vec::new();
        let mut strip_cell_types = Vec::new();
        model.get_cell_styles_in(frame.sheet, strip, &mut strip_styles);
        model.get_formatted_cell_values_in(frame.sheet, strip, &mut strip_values);
        model.get_cell_types_in(frame.sheet, strip, &mut strip_cell_types);

        let mut pane_styles = pane_buf.styles.take();
        let mut pane_values = pane_buf.values.take();
        let mut pane_cell_types = pane_buf.cell_types.take();
        splice_strip_into(&mut pane_styles, &mut strip_styles, range, strip);
        splice_strip_into(&mut pane_values, &mut strip_values, range, strip);
        splice_strip_into(&mut pane_cell_types, &mut strip_cell_types, range, strip);

        if let Some(strip_rect) = frame.range_rect(strip) {
            self.painter
                .rect_fill(strip_rect, PaintColor::from_theme_str(&theme.cell_bg));
        }

        let mut slots = self.frame_cache.text_slots.take();
        slots.clear();
        // Walk strip cells only. `apply_blit_shift` rotated the kept-band
        // entries (still `Some(...)`) into their new pane indices, so a
        // full-pane walk would re-`take` and re-paint the kept band on top
        // of pixels the painter blit already placed — wasting the entire
        // win. `PaneCells::for_strip` narrows the slot slices up front.
        for slot in PaneCells::for_strip(&pane, frame, strip) {
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
        pane_buf.styles.set(pane_styles);
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
        pane_buf.values.set(pane_values);
        pane_buf.cell_types.set(pane_cell_types);
        pane_buf.range.set(Some(range));
        self.frame_cache.text_slots.set(slots);
        self.frame_cache.text_lines.set(text_lines);

        let mut fps = frame.pane_fingerprints.get();
        fps[pane_idx] = 0;
        frame.pane_fingerprints.set(fps);
    }
}

/// Identify a single-axis scroll between two pane RCRanges. Returns the
/// scroll axis when one axis's endpoints differ and the other axis is
/// identical, with both extents preserved. Used by `render_pane`'s Stage
/// 3.3 detect to switch into the strip-fetch branch.
fn infer_shift_axis(prev: RCRange, new: RCRange) -> Option<Axis> {
    let rows_same = prev.r1 == new.r1 && prev.r2 == new.r2;
    let cols_same = prev.c1 == new.c1 && prev.c2 == new.c2;
    let row_extent_same = (new.r2 - new.r1) == (prev.r2 - prev.r1);
    let col_extent_same = (new.c2 - new.c1) == (prev.c2 - prev.c1);
    if !row_extent_same || !col_extent_same {
        return None;
    }
    match (rows_same, cols_same) {
        (true, false) => Some(Axis::Column),
        (false, true) => Some(Axis::Row),
        _ => None,
    }
}

/// Slice of `new` lying outside `prev` along the scroll axis. Returns
/// `None` if the ranges are identical along `axis` (delta == 0). Under
/// `try_blit` qualification, `|delta| < extent` is guaranteed so the
/// no-overlap path is defensive only.
fn compute_strip(prev: RCRange, new: RCRange, axis: Axis) -> Option<RCRange> {
    match axis {
        Axis::Row => {
            if new.r2 < prev.r1 || new.r1 > prev.r2 {
                return Some(new);
            }
            if new.r1 < prev.r1 {
                Some(RCRange {
                    r1: new.r1,
                    r2: prev.r1 - 1,
                    c1: new.c1,
                    c2: new.c2,
                })
            } else if new.r2 > prev.r2 {
                // Includes `prev.r2` (not `prev.r2 + 1`) because that row
                // was the overflow row in prev — its pixels were off-canvas
                // and weren't shifted by the blit, so its on-canvas position
                // in new needs a fresh paint.
                Some(RCRange {
                    r1: prev.r2,
                    r2: new.r2,
                    c1: new.c1,
                    c2: new.c2,
                })
            } else {
                None
            }
        }
        Axis::Column => {
            if new.c2 < prev.c1 || new.c1 > prev.c2 {
                return Some(new);
            }
            if new.c1 < prev.c1 {
                Some(RCRange {
                    r1: new.r1,
                    r2: new.r2,
                    c1: new.c1,
                    c2: prev.c1 - 1,
                })
            } else if new.c2 > prev.c2 {
                // Mirror of the Row down-scroll case: prev.c2 was the
                // overflow column whose pixels were off-canvas.
                Some(RCRange {
                    r1: new.r1,
                    r2: new.r2,
                    c1: prev.c2,
                    c2: new.c2,
                })
            } else {
                None
            }
        }
    }
}

/// Move the freshly-fetched strip cells into `pane_buf` at the indices
/// corresponding to their `(row, col)` within the new pane range. Drains
/// `strip_buf` via `Option::take` so callers can drop the scratch Vec
/// after the splice without leaking the inner values.
fn splice_strip_into<T>(
    pane_buf: &mut [Option<T>],
    strip_buf: &mut [Option<T>],
    pane_range: RCRange,
    strip_range: RCRange,
) {
    let pane_cols = (pane_range.c2 - pane_range.c1 + 1) as usize;
    let strip_cols = (strip_range.c2 - strip_range.c1 + 1) as usize;
    let strip_rows = (strip_range.r2 - strip_range.r1 + 1) as usize;
    debug_assert_eq!(strip_buf.len(), strip_rows * strip_cols);
    let row_offset = (strip_range.r1 - pane_range.r1) as usize;
    let col_offset = (strip_range.c1 - pane_range.c1) as usize;
    for r in 0..strip_rows {
        for c in 0..strip_cols {
            let strip_idx = r * strip_cols + c;
            let pane_idx_inner = (row_offset + r) * pane_cols + (col_offset + c);
            pane_buf[pane_idx_inner] = strip_buf[strip_idx].take();
        }
    }
}
