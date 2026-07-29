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
use self::fingerprint::{RepaintPlan, plan_pane_repaint};
use self::text::TextPaint;
use crate::CellContentQuery;
use crate::chrome::{BlitPlan, Chrome, FrameKindTag, PaneRegion};
use crate::orchestrator::PaneVerdict;
use crate::painter::{PaintColor, Painter};
use crate::renderer::RendererCore;
use crate::renderer::blit_work::{BlitPaneWork, widen_blit_strip_to_pixel_clip};
use crate::renderer::cache::PaneShiftPrep;
use crate::renderer::cf_types::CfDecorationPaint;
use crate::signal::RowSpan;
use crate::theme::CanvasTheme;
use crate::types::coord::RCRange;

/// The four parallel strip buffers (styles / values / cell-types /
/// decorations) that flow through the blit strip machinery together. Grouped
/// as one alias so `paint_strip_from_fetched` can hand them back for parking
/// without tripping `clippy::type_complexity`.
type StripBuffers = (
    Vec<Fetched<CellStyle>>,
    Vec<Fetched<String>>,
    Vec<Fetched<CellKind>>,
    Vec<Fetched<CellDecoration>>,
);

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
        let pane_buf = self.pane_cache.pane(pane);

        let Some(range) = pane.range(frame) else {
            // Pane became empty (e.g. freeze removed on this axis). Forget the
            // cached range so a future re-grow refetches. The painted tree
            // needs no explicit reset: a later re-grow builds a scratch tree
            // for a real range, and range-in-digest means it can't collide
            // with whatever stale tree sits in `painted`.
            pane_buf.range.set(None);
            return false;
        };

        let theme = &frame.theme;
        let reuses_slots = frame.kind.reuses_slots();

        // On reused-slot frames, prior pixels are still visible until this
        // method paints over them. A transient BridgeFailed fetch is therefore
        // an instruction to hold the old pane atomically: no clear, no
        // fingerprint commit, and no cache poisoning. Keep the prior buffers
        // parked aside while the new fetch uses fresh scratch vectors. Fresh
        // frames keep the old allocation-reuse path because there are no prior
        // pane pixels to preserve.
        // A blit frame that could not shift this pane already fetched and
        // bridge-validated this exact range in `unshiftable_pane_is_safe`.
        // Adopting it is the whole point: that pane is the one place the
        // renderer would otherwise cross the bridge twice for the same cells,
        // on the most expensive frame it produces.
        let staged = self.take_validated_pane_fetch(pane, range, frame);
        let staged_fetch = staged.is_some();

        let previous_buffers = if reuses_slots && !staged_fetch {
            Some((
                pane_buf.styles.take(),
                pane_buf.values.take(),
                pane_buf.cell_types.take(),
                pane_buf.decorations.take(),
            ))
        } else {
            None
        };

        // Bulk-fetch styles + formatted values for the whole rectangular
        // range. UserModel default impls loop the per-cell accessors (no perf
        // change); JsBackedModel will override (W5) and collapse each to one
        // JS call per pane.
        let (mut pane_styles, mut pane_values, mut pane_cell_types, mut pane_decorations) =
            match staged {
                Some(buffers) => buffers,
                None => match &previous_buffers {
                    Some((styles, values, cell_types, decorations)) => (
                        Vec::with_capacity(styles.len()),
                        Vec::with_capacity(values.len()),
                        Vec::with_capacity(cell_types.len()),
                        Vec::with_capacity(decorations.len()),
                    ),
                    None => (
                        pane_buf.styles.take(),
                        pane_buf.values.take(),
                        pane_buf.cell_types.take(),
                        pane_buf.decorations.take(),
                    ),
                },
            };
        if !staged_fetch {
            model.get_cell_styles_in(frame.sheet, range, &mut pane_styles);
            model.get_formatted_cell_values_in(frame.sheet, range, &mut pane_values);
            model.get_cell_types_in(frame.sheet, range, &mut pane_cell_types);
            model.get_cell_decorations_in(frame.sheet, range, &mut pane_decorations);
            self.trace_fetch(range);
        }

        // A staged fetch was already bridge-validated by the preflight, and
        // `previous_buffers` is `None` on that path, so there is nothing to
        // restore and nothing to re-scan.
        if !staged_fetch
            && reuses_slots
            && (has_bridge_failure(&pane_styles)
                || has_bridge_failure(&pane_values)
                || has_bridge_failure(&pane_cell_types)
                || has_bridge_failure(&pane_decorations))
        {
            if let Some((styles, values, cell_types, decorations)) = previous_buffers {
                pane_buf.styles.set(styles);
                pane_buf.values.set(values);
                pane_buf.cell_types.set(cell_types);
                pane_buf.decorations.set(decorations);
            }
            // Preflight rejected this frame's fetch — the painted tree is
            // never touched on this path (no `take`/`store` above), so a
            // run of consecutive failures leaves it exactly as the last
            // successful paint left it.
            self.trace_pane(pane, PaneVerdict::Held);
            return true;
        }

        // Fingerprint paint-skip: same content as the previous frame
        // -> canvas pixels are still correct, skip the five-pass walk. Bulk
        // fetch above is unconditional now — content changes that don't
        // raise CONTENT (e.g. recalc triggered by an upstream edit a
        // caller forgot to mark) are detected here via fingerprint
        // mismatch, not assumed away by a geometric early-exit.
        //
        // `rebuild_scratch` writes the full pane -> row -> cell tree into
        // `pane_buf.fingerprint`'s persistent `scratch` slot (reusing its
        // warm `Vec` capacity rather than allocating a fresh tree every
        // frame). `PaneCache` (via `pane_buf.fingerprint`) owns the
        // painted-vs-scratch compare — `Chrome` carries none of this state
        // across frames anymore.
        pane_buf.fingerprint.rebuild_scratch(
            &pane_styles,
            &pane_values,
            &pane_cell_types,
            &pane_decorations,
            range,
        );

        // A `Fresh` frame has no prior valid pixels to partially preserve
        // (nothing to diff against, nothing to leave alone), so it always
        // takes the unconditional full repaint — `plan_pane_repaint` never
        // runs for it.
        if !reuses_slots {
            pane_buf.fingerprint.commit();
            self.trace_pane(pane, PaneVerdict::Full);
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
            return false;
        }

        // A `SlotsReuse` frame diffs this frame's freshly rebuilt `scratch`
        // tree against the last-committed `painted` tree once and dispatches
        // uniformly on the verdict. `plan_pane_repaint`'s own first line is
        // the digest-equal Skip fast path — no separate marker check is
        // needed ahead of it (range is baked into the digest, so a stale
        // painted tree can't spuriously Skip; see `PaneFingerprintState`).
        let plan = pane_buf.fingerprint.with_trees(plan_pane_repaint);
        self.trace_pane(pane, PaneVerdict::from(&plan));

        match plan {
            RepaintPlan::Skip => {
                // Content-identical: no paint. `commit` still swaps the
                // just-rebuilt scratch into `painted` (cheap, no allocation)
                // so next frame's compare reads the freshest tree, then the
                // buffers are parked back for reuse.
                pane_buf.fingerprint.commit();
                pane_buf.styles.set(pane_styles);
                pane_buf.values.set(pane_values);
                pane_buf.cell_types.set(pane_cell_types);
                // Set decorations back too: a later blit `prepare_shift`
                // rotates this buffer against the cached `Some(range)`; an
                // empty vec would misalign indices and yield wrong decorations.
                pane_buf.decorations.set(pane_decorations);
                pane_buf.range.set(Some(range));
            }
            RepaintPlan::Rows(spans) => {
                // Safe, scoped repaint: clear + five-pass paint each
                // band from the buffers this call already bulk-fetched
                // above — no second model query.
                pane_buf.fingerprint.commit();
                self.paint_pane_row_spans(
                    pane,
                    range,
                    frame,
                    theme,
                    &spans,
                    pane_styles,
                    pane_values,
                    pane_cell_types,
                    pane_decorations,
                );
            }
            RepaintPlan::Full => {
                // Content changed on a reused-canvas frame: clear the pane
                // bg so cells whose data just disappeared don't leave
                // stale pixels.
                if let Some(pane_rect) = frame.range_rect(range) {
                    self.painter
                        .rect_fill(pane_rect, PaintColor::from_theme_str(&theme.cell_bg));
                }
                pane_buf.fingerprint.commit();
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
        }
        false
    }

    /// Shared paint tail for `render_pane`, `render_pane_strip`, and the
    /// row-band multi-span path: the five deferred passes (bg -> CF
    /// decoration -> grid borders -> explicit borders -> text) over
    /// `cells`, reading from the four bulk buffers *by mutable borrow*
    /// rather than by value. Separate from `paint_pane_cells` so
    /// a multi-span caller can invoke this once per span against the SAME
    /// four buffers without re-taking them from `pane_buf` or parking them
    /// back in between — ownership and the take/park lifecycle live
    /// entirely with the caller. Pass order is load-bearing — see the doc
    /// on `render_pane` for why bg precedes borders precedes text.
    #[allow(clippy::too_many_arguments)]
    fn paint_cells_pass(
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

    /// Full-pane single-walk entry: take/park boilerplate around exactly
    /// one [`Self::paint_cells_pass`] call. Used by `render_pane`'s
    /// `Fresh`-frame path and `RepaintPlan::Full`, and by
    /// `render_pane_strip`'s single-strip walk.
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
        mut pane_decorations: Vec<Fetched<CellDecoration>>,
    ) {
        self.paint_cells_pass(
            cells,
            range,
            theme,
            &mut pane_styles,
            &mut pane_values,
            &mut pane_cell_types,
            &mut pane_decorations,
        );

        let pane_buf = self.pane_cache.pane(pane);
        pane_buf.styles.set(pane_styles);
        pane_buf.values.set(pane_values);
        pane_buf.cell_types.set(pane_cell_types);
        pane_buf.decorations.set(pane_decorations);
        pane_buf.range.set(Some(range));
    }

    /// Row-band repaint: clear + five-pass paint each `RowSpan` in
    /// `spans`, all from ONE take of the buffers `render_pane`'s own
    /// upfront bulk-fetch already populated this frame — no second model
    /// query, however many spans. The four buffers thread through every
    /// span via `&mut` (never re-taken from `pane_buf`, never parked
    /// mid-loop) and are parked back exactly once, after the last span —
    /// mirroring `paint_pane_cells`'s single-parking-point discipline, just
    /// spread over N paint passes instead of one.
    ///
    /// Each band clears independently, immediately before its own paint —
    /// not one clear covering every span's extent — so untouched rows
    /// between two spans never lose their pixels. Bands are always
    /// full pane width (`range.c1..=range.c2`), never narrowed by column:
    /// text overflow can bleed horizontally into a neighbour cell, so a
    /// column-narrow clear could leave stale overflow behind.
    #[allow(clippy::too_many_arguments)]
    fn paint_pane_row_spans(
        &self,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
        theme: &CanvasTheme,
        spans: &[RowSpan],
        mut pane_styles: Vec<Fetched<CellStyle>>,
        mut pane_values: Vec<Fetched<String>>,
        mut pane_cell_types: Vec<Fetched<CellKind>>,
        mut pane_decorations: Vec<Fetched<CellDecoration>>,
    ) {
        for span in spans {
            let band = RCRange {
                r1: span.r1,
                c1: range.c1,
                r2: span.r2,
                c2: range.c2,
            };
            if let Some(band_rect) = frame.range_rect(band) {
                self.painter
                    .rect_fill(band_rect, PaintColor::from_theme_str(&theme.cell_bg));
            }
            // `range` (the whole pane's fetch range), not `band` — the four
            // buffers are dense over the whole pane, same convention
            // `render_pane_strip` uses when it narrows `PaneCells` to a
            // strip but still indexes against the pane's full range.
            self.paint_cells_pass(
                PaneCells::for_strip(&pane, frame, band),
                range,
                theme,
                &mut pane_styles,
                &mut pane_values,
                &mut pane_cell_types,
                &mut pane_decorations,
            );
        }

        let pane_buf = self.pane_cache.pane(pane);
        pane_buf.styles.set(pane_styles);
        pane_buf.values.set(pane_values);
        pane_buf.cell_types.set(pane_cell_types);
        pane_buf.decorations.set(pane_decorations);
        pane_buf.range.set(Some(range));
    }

    /// Paint bg + borders for one resolved `CellPaint`. Used by
    /// `repaint_active_cell` where a single-cell batch is not worth the
    /// overhead.
    pub(super) fn paint_cell(&self, p: &CellPaint, theme: &CanvasTheme) {
        self.paint_bg(p, theme);
        self.paint_borders(p, theme);
    }

    /// Whole-frame blit preflight: fetch and bridge-validate every shifted
    /// pane's revealed strip BEFORE any pixel is blitted. Returns `true` if the
    /// caller may proceed to shift + paint, `false` if ANY strip fetch failed —
    /// in which case the caller must treat this frame as a no-op (do not blit,
    /// do not paint). Shifting pixels before the fetch is known good is exactly
    /// the bug this closes: a failed fetch would otherwise leave the revealed
    /// strip showing stale, now-misplaced pixels with nothing to repaint them.
    ///
    /// Pure classification only (`classify_shift`, no cache rotation), so an
    /// aborted frame leaves every pane's cache, pixels, and fingerprint
    /// untouched; the rotation is deferred to `render_grid_blit`'s paint pass.
    /// On success each shifted pane's validated strip is stashed in its
    /// `blit_stage` slot for `render_pane_blit` to paint from without a second
    /// model round-trip.
    pub fn prefetch_blit_strips(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        // Clear readiness left over from a prior frame so a stale slot can
        // never feed this frame's paint.
        self.clear_blit_stage_readiness();
        for pane in frame.stale_panes.regions() {
            let Some(work) = self.plan_preflight_strip(frame, plan, pane) else {
                // Not shift-and-strippable, so `render_grid_blit` hands this
                // pane to the full `render_pane` — but only AFTER
                // `paint_grid_blit` has already shifted its pixels.
                if pane.range(frame).is_some() {
                    let cold_cache = self.pane_cache.pane(pane).range.get().is_none();
                    self.trace_blit_fallback(pane, cold_cache);
                }
                if !self.unshiftable_pane_is_safe(model, pane, frame) {
                    self.clear_blit_stage_readiness();
                    self.trace_frame_held(pane);
                    return false;
                }
                continue;
            };
            let stage = &self.blit_stage[pane as usize];

            let mut styles = stage.styles.take();
            let mut values = stage.values.take();
            let mut cell_types = stage.cell_types.take();
            let mut decorations = stage.decorations.take();
            model.get_cell_styles_in(frame.sheet, work.strip_range, &mut styles);
            model.get_formatted_cell_values_in(frame.sheet, work.strip_range, &mut values);
            model.get_cell_types_in(frame.sheet, work.strip_range, &mut cell_types);
            model.get_cell_decorations_in(frame.sheet, work.strip_range, &mut decorations);
            self.trace_fetch(work.strip_range);
            let failed = has_bridge_failure(&styles)
                || has_bridge_failure(&values)
                || has_bridge_failure(&cell_types)
                || has_bridge_failure(&decorations);

            stage.strip.set(Some(work.strip_range));
            stage.styles.set(styles);
            stage.values.set(values);
            stage.cell_types.set(cell_types);
            stage.decorations.set(decorations);

            if failed {
                // One failed strip rejects the WHOLE frame atomically. Reset
                // every slot's readiness so a partially-populated stage can
                // never paint on a later call.
                self.clear_blit_stage_readiness();
                self.trace_frame_held(pane);
                return false;
            }
            stage.ready.set(true);
        }
        true
    }

    /// Drop every staged fetch's claim to be consumable — both the strip
    /// stagings and the whole-pane ones. Called at preflight entry and on any
    /// abort, so a partially-populated stage can never feed a later paint.
    fn clear_blit_stage_readiness(&self) {
        for stage in &self.blit_stage {
            stage.ready.set(false);
            stage.full_pane.set(None);
        }
    }

    /// Adopt the full-range fetch the preflight already validated for `pane`,
    /// if there is one. Hands the pane's own (about-to-be-overwritten) buffers
    /// back to the stage so neither pool loses its warm capacity.
    ///
    /// Two guards, both load-bearing: the stage outlives the frame, so without
    /// the `Blitted` check a later ordinary paint could adopt a stale fetch,
    /// and without the range check a same-geometry frame could adopt one for
    /// the wrong address space.
    fn take_validated_pane_fetch(
        &self,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
    ) -> Option<StripBuffers> {
        if !matches!(frame.kind, FrameKindTag::Blitted) {
            return None;
        }
        let stage = &self.blit_stage[pane as usize];
        if stage.full_pane.take() != Some(range) {
            return None;
        }
        let pane_buf = self.pane_cache.pane(pane);
        Some((
            stage.styles.replace(pane_buf.styles.take()),
            stage.values.replace(pane_buf.values.take()),
            stage.cell_types.replace(pane_buf.cell_types.take()),
            stage.decorations.replace(pane_buf.decorations.take()),
        ))
    }

    /// The shift-and-strip half of the preflight's per-pane classification:
    /// `Some(work)` when this pane's revealed strip can be computed (and so
    /// pre-fetched), `None` when the blit machinery will fall back to a full
    /// `render_pane` for it — empty live range, `MissingCache`,
    /// `IncompatibleRange`, or the zero-delta guard. Mirrors
    /// `render_grid_blit`'s own dispatch, but via `classify_shift` so nothing
    /// is mutated before the frame is known good.
    fn plan_preflight_strip(
        &self,
        frame: &Chrome,
        plan: &BlitPlan,
        pane: PaneRegion,
    ) -> Option<BlitPaneWork> {
        let new_range = pane.range(frame)?;
        let PaneShiftPrep::Shifted {
            prev_range,
            new_range,
        } = self
            .pane_cache
            .pane(pane)
            .classify_shift(new_range, plan.axis)
        else {
            return None;
        };
        let address_work = self
            .pane_cache
            .plan_blit_pane(prev_range, new_range, plan.axis)?;
        Some(widen_blit_strip_to_pixel_clip(
            frame,
            plan,
            pane,
            address_work,
        ))
    }

    /// Decide whether the frame may still proceed given a `stale_panes` pane
    /// the preflight could not stage a strip for. `true` = keep going, `false`
    /// = abandon the whole frame (no shift, no paint).
    ///
    /// Why this needs deciding at all: `paint_grid_blit` shifts pixels for
    /// every rect in `plan.shifts` — including this pane's — and only then does
    /// `render_grid_blit` route this pane to the full `render_pane`. On a
    /// `Blitted` frame `FrameKindTag::reuses_slots()` is true, so a
    /// `BridgeFailed` fetch inside `render_pane` makes it hold the prior
    /// buffers and return WITHOUT painting. The pixels have already moved by
    /// then, so the pane is left showing stale, misplaced content — the same
    /// failure the strip preflight exists to prevent, reached by a different
    /// route.
    /// Validates the pane's own full-range fetch rather than unconditionally
    /// abandoning: `IncompatibleRange` is the ordinary jump-scroll verdict, so
    /// a blanket `false` would drop a frame on every page-down for a hazard
    /// that only exists when the bridge is actually failing. The duplicate
    /// fetch (`render_pane` repeats it moments later) buys back every healthy
    /// frame, and lands only on this already-slow full-repaint path.
    ///
    /// Borrows the pane's idle `blit_stage` vectors as scratch — it is staging
    /// no strip this frame, so they are free, and `ready` stays `false` so
    /// nothing downstream mistakes them for a validated strip.
    fn unshiftable_pane_is_safe(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        frame: &Chrome,
    ) -> bool {
        // No live range -> no pixels of its own to strand.
        let Some(range) = pane.range(frame) else {
            return true;
        };

        let stage = &self.blit_stage[pane as usize];
        let mut styles = stage.styles.take();
        let mut values = stage.values.take();
        let mut cell_types = stage.cell_types.take();
        let mut decorations = stage.decorations.take();
        model.get_cell_styles_in(frame.sheet, range, &mut styles);
        model.get_formatted_cell_values_in(frame.sheet, range, &mut values);
        model.get_cell_types_in(frame.sheet, range, &mut cell_types);
        model.get_cell_decorations_in(frame.sheet, range, &mut decorations);
        self.trace_fetch(range);
        let ok = !(has_bridge_failure(&styles)
            || has_bridge_failure(&values)
            || has_bridge_failure(&cell_types)
            || has_bridge_failure(&decorations));

        stage.styles.set(styles);
        stage.values.set(values);
        stage.cell_types.set(cell_types);
        stage.decorations.set(decorations);
        if ok {
            // Hand this fetch to the `render_pane` that `render_grid_blit` is
            // about to run for this pane. On failure the frame is abandoned, so
            // there is nothing to hand over.
            stage.full_pane.set(Some(range));
        }
        ok
    }

    /// Blit-frame entry: paint only the revealed strip. The cache rotation
    /// (`prepare_shift`) and the strip/axis/clip computation already happened in
    /// `render_grid_blit` — this consumes the precomputed [`BlitPaneWork`] and
    /// paints the strip cells; kept-band cells keep their blitted pixels and
    /// are skipped because the walk is narrowed to the strip
    /// (`PaneCells::for_strip`).
    ///
    /// The strip cells were already fetched and bridge-validated by
    /// `prefetch_blit_strips` (the whole-frame preflight that runs before any
    /// pixel is shifted). Those pre-fetched buffers are threaded straight into
    /// the paint tail here — no second model round-trip. If this pane has no
    /// staged fetch (e.g. a direct `render_grid_blit` call in a test that
    /// skipped the preflight), it falls back to the combined
    /// fetch-and-paint `render_pane_strip`.
    pub fn render_pane_blit(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        work: &BlitPaneWork,
    ) {
        let pane = work.pane;
        let pane_buf = self.pane_cache.pane(pane);
        let Some(range) = pane.range(frame) else {
            // Same empty-pane rationale as `render_pane`'s early return.
            pane_buf.range.set(None);
            return;
        };

        let stage = &self.blit_stage[pane as usize];
        if stage.ready.take() {
            debug_assert_eq!(stage.strip.get(), Some(work.strip_range));
            // The preflight already charged this strip's fetch to the trace.
            self.trace_pane(pane, PaneVerdict::Strip);
            let strip_styles = stage.styles.take();
            let strip_values = stage.values.take();
            let strip_cell_types = stage.cell_types.take();
            let strip_decorations = stage.decorations.take();
            let (styles, values, cell_types, decorations) = self.paint_strip_from_fetched(
                pane,
                range,
                frame,
                work.strip_range,
                strip_styles,
                strip_values,
                strip_cell_types,
                strip_decorations,
            );
            // Park the (now drained) strip buffers back into the stage so the
            // next preflight reuses their warm capacity.
            stage.styles.set(styles);
            stage.values.set(values);
            stage.cell_types.set(cell_types);
            stage.decorations.set(decorations);
        } else {
            self.render_pane_strip(model, pane, range, frame, work.strip_range);
        }
    }

    /// Damage-frame entry for one pane: repaint only the full-width row
    /// bands in `spans`, via the same strip machinery the blit path uses.
    /// Kept rows keep their pixels; each band fetch splices into the pane
    /// buffers and zeroes the pane fingerprint (`render_pane_strip`).
    ///
    /// Returns `true` when this pane's work was held rather than committed:
    /// either the range-mismatch demotion forwards `render_pane`'s own
    /// verdict, or a span's strip fetch failed. A held span stops the loop
    /// immediately — the retry re-runs the pane's original spans, so there
    /// is no benefit to splicing past a hold, only risk of a second failure
    /// mid-pane.
    pub fn render_pane_damage(
        &self,
        model: &dyn CellContentQuery,
        frame: &Chrome,
        pane: PaneRegion,
        spans: &[RowSpan],
    ) -> bool {
        let pane_buf = self.pane_cache.pane(pane);
        let Some(range) = pane.range(frame) else {
            // Same empty-pane rationale as `render_pane`'s early return.
            pane_buf.range.set(None);
            return false;
        };
        // `splice_strip_into` indexes the cached pane buffers; they are
        // only aligned when the cached range matches this frame's. A
        // mismatch (e.g. partial post-blit buffers) demotes the pane to
        // the full walk instead of splicing at wrong indices.
        if pane_buf.range.get() != Some(range) {
            return self.render_pane(model, pane, frame);
        }
        for span in spans {
            let r1 = span.r1.max(range.r1);
            let r2 = span.r2.min(range.r2);
            if r1 > r2 {
                continue;
            }
            let band = RCRange {
                r1,
                c1: range.c1,
                r2,
                c2: range.c2,
            };
            if self.render_pane_strip(model, pane, range, frame, band) {
                return true;
            }
        }
        false
    }

    /// Stage 3.3 strip path (combined fetch + paint): the freshly-revealed
    /// strip subrange (`strip`, precomputed in `render_grid_blit`) is fetched
    /// from the model and painted; kept-band cells are skipped because the
    /// walk is narrowed to the strip (`PaneCells::for_strip`). Used by the
    /// damage path and as the fall-back when the blit preflight did not
    /// pre-fetch this pane's strip (see `render_pane_blit`).
    ///
    /// The painted-fingerprint tree is deliberately NOT touched here: a strip
    /// paint just doesn't `commit`, so `painted` keeps the last full paint's
    /// range/digest. Because range is folded into the digest, the next
    /// `SlotsReuse` frame's `plan_pane_repaint` sees a range mismatch (or, on
    /// a genuine round-trip back to a previous range with unchanged content, a
    /// correct `Skip`) with no manual marker.
    ///
    /// Atomic across all four strip buffers: a transient `BridgeFailed` on
    /// any one of them means this frame's strip fetch cannot be trusted, so
    /// the preflight below rejects the whole update before anything is
    /// spliced, cleared, or painted — mirroring `render_pane`'s own
    /// preflight for the full-pane fetch. On rejection the pane's cached
    /// buffers, on-screen pixels, and `range` are left exactly as they were;
    /// only the `FrameCache` scratch vecs are parked back for reuse next frame.
    ///
    /// Returns `true` exactly on the held (bridge-failure) branch below;
    /// `false` once the strip actually paints.
    fn render_pane_strip(
        &self,
        model: &dyn CellContentQuery,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
        strip: RCRange,
    ) -> bool {
        // Strip-fetch scratch reused from `FrameCache` (take/set rhythm),
        // not `Vec::new()` per frame — `paint_strip_from_fetched` drains
        // these into the pane buffers and hands them back for parking. The
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
        self.trace_fetch(strip);

        if has_bridge_failure(&strip_styles)
            || has_bridge_failure(&strip_values)
            || has_bridge_failure(&strip_cell_types)
            || has_bridge_failure(&strip_decorations)
        {
            // Preflight rejected this strip fetch — park the scratch vecs
            // for reuse next frame and bail before touching `pane_buf` at
            // all: no splice, no clear, no paint. A run of consecutive
            // failures leaves the pane exactly as the last successful strip
            // (or full paint) left it.
            self.frame_cache.strip_styles.set(strip_styles);
            self.frame_cache.strip_values.set(strip_values);
            self.frame_cache.strip_cell_types.set(strip_cell_types);
            self.frame_cache.strip_decorations.set(strip_decorations);
            self.trace_pane(pane, PaneVerdict::Held);
            return true;
        }
        self.trace_pane(pane, PaneVerdict::Strip);

        let (strip_styles, strip_values, strip_cell_types, strip_decorations) = self
            .paint_strip_from_fetched(
                pane,
                range,
                frame,
                strip,
                strip_styles,
                strip_values,
                strip_cell_types,
                strip_decorations,
            );
        self.frame_cache.strip_styles.set(strip_styles);
        self.frame_cache.strip_values.set(strip_values);
        self.frame_cache.strip_cell_types.set(strip_cell_types);
        self.frame_cache.strip_decorations.set(strip_decorations);
        false
    }

    /// Paint tail shared by `render_pane_strip` (combined fetch) and
    /// `render_pane_blit`'s pre-fetched path: splice the already-fetched +
    /// bridge-validated strip buffers into the cached pane buffers, clear the
    /// strip's pixels, and repaint only the strip cells. Returns the (now
    /// drained) strip buffers so the caller can park them back into whichever
    /// pool it took them from — `FrameCache` scratch for the combined path,
    /// the per-pane `blit_stage` for the pre-fetched path.
    ///
    /// Does NOT re-check `has_bridge_failure`: callers validate before calling
    /// (the combined path in `render_pane_strip`, the whole-frame preflight in
    /// `prefetch_blit_strips`). Does NOT touch the painted-fingerprint tree —
    /// see `render_pane_strip`'s doc for why range-in-digest makes that safe.
    #[allow(clippy::too_many_arguments)]
    fn paint_strip_from_fetched(
        &self,
        pane: PaneRegion,
        range: RCRange,
        frame: &Chrome,
        strip: RCRange,
        mut strip_styles: Vec<Fetched<CellStyle>>,
        mut strip_values: Vec<Fetched<String>>,
        mut strip_cell_types: Vec<Fetched<CellKind>>,
        mut strip_decorations: Vec<Fetched<CellDecoration>>,
    ) -> StripBuffers {
        let theme = &frame.theme;
        let pane_buf = self.pane_cache.pane(pane);

        let mut pane_styles = pane_buf.styles.take();
        let mut pane_values = pane_buf.values.take();
        let mut pane_cell_types = pane_buf.cell_types.take();
        let mut pane_decorations = pane_buf.decorations.take();
        splice_strip_into(&mut pane_styles, &mut strip_styles, range, strip);
        splice_strip_into(&mut pane_values, &mut strip_values, range, strip);
        splice_strip_into(&mut pane_cell_types, &mut strip_cell_types, range, strip);
        splice_strip_into(&mut pane_decorations, &mut strip_decorations, range, strip);

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

        (
            strip_styles,
            strip_values,
            strip_cell_types,
            strip_decorations,
        )
    }
}

fn has_bridge_failure<T>(items: &[Fetched<T>]) -> bool {
    items.iter().any(Fetched::is_bridge_failed)
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
