//! Frame dispatch and state aggregator. Backend-agnostic; the wasm-bound
//! `IronCanvas` facade in `iron-canvas-web` owns an
//! `Orchestrator<WebSurface, Rc<dyn CanvasModel>>` and delegates every
//! setter, query, and paint call here.
//!
//! `paint_if_dirty` drains both layers' typed `GridSignals` and picks one
//! of four `PaintRegime` arms via `decide` (cheapness-ordered). Each arm
//! runs a `Chrome::next(.., FramePath::*)` walk through the matching
//! `LayerBase` paint method. The query API (`hit_test`, `cell_rect`,
//! `resize_handle_at`, `autofill_handle`) reads `last_frame`, so hits
//! agree with painted pixels by construction.

use std::cell::Cell;

use serde::{Deserialize, Serialize};

use crate::chrome::{BlitPlan, Chrome, FrameKindTag, FramePath, FrameValidity, PaneRegionMask};
use crate::decoration::{
    autofill::AutofillLayer, clipboard::ClipboardLayer, formula_refs::FormulaRefsLayer,
    point_mode::PointModeLayer, selection::SelectionLayer, Layer,
};
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::geometry::CanvasSize;
use crate::layer::{LayerBase, Surface};
use crate::painter::BlitPainter;
use crate::render_overlays::RenderOverlays;
use crate::renderer::{GridRenderer, OverlayRenderer};
use crate::signal::GridSignals;
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use crate::types::ui::{HitTest, ResizeTarget};
use crate::CanvasModel;

/// Named verdict of `paint_if_dirty`'s dispatch. Each variant aligns 1:1
/// with a `paint_*` method; the regime carries everything that method
/// needs (mask, dirty bits) so `paint_if_dirty` is pure pattern-destructure.
/// Variants align with `FramePath`: `SlotsReuse` and `Fresh` here map to
/// `FramePath::SlotsReuse` and `FramePath::Fresh` inside `Chrome::next`.
pub enum PaintRegime {
    Overlay,
    Viewport(BlitPlan),
    SlotsReuse {
        mask: PaneRegionMask,
        signals: GridSignals,
    },
    Fresh(GridSignals),
}

/// Data-free public mirror of `PaintRegime`. Stamped by `paint_if_dirty`
/// into `Orchestrator.last_regime` so out-of-engine consumers (the
/// recording pipeline) can attribute each captured frame to a regime
/// without seeing the regime's inner data (`BlitPlan`, `PaneRegionMask`,
/// `GridSignals`). Serializes with snake_case variant names to match the
/// `.icr` JSON-lines schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaintRegimeTag {
    Overlay,
    Viewport,
    SlotsReuse,
    Fresh,
}

impl From<&PaintRegime> for PaintRegimeTag {
    fn from(r: &PaintRegime) -> Self {
        match r {
            PaintRegime::Overlay => PaintRegimeTag::Overlay,
            PaintRegime::Viewport(_) => PaintRegimeTag::Viewport,
            PaintRegime::SlotsReuse { .. } => PaintRegimeTag::SlotsReuse,
            PaintRegime::Fresh(_) => PaintRegimeTag::Fresh,
        }
    }
}

pub struct Orchestrator<S, M>
where
    S: Surface,
    S::P: BlitPainter,
    M: CanvasModel,
{
    pub(crate) grid: LayerBase<S, GridRenderer<S::P>>,
    pub(crate) overlay: LayerBase<S, OverlayRenderer<S::P>>,
    theme: CanvasTheme,
    selection: SelectionLayer,
    autofill: AutofillLayer,
    clipboard: ClipboardLayer,
    point_mode: PointModeLayer,
    formula_refs: FormulaRefsLayer,
    model: Option<M>,
    last_frame: Option<Chrome>,
    /// Logical (CSS) canvas size; written by `resize`, read when building
    /// the next `Chrome`.
    size: CanvasSize,
    /// Typed cell-content-changed signal accumulated since the last paint.
    /// Bits name the panes whose cached buffers are stale and must refetch.
    /// `decide` routes a non-empty mask through the SlotsReuse arm when
    /// the viewport is otherwise reusable. Reset to `EMPTY` at the end of
    /// every `paint_if_dirty`.
    pending_content: PaneRegionMask,
    /// Last regime `paint_if_dirty` dispatched. Stamped after `decide`,
    /// read by the recording pipeline via `last_regime()`. `None` before
    /// the first paint. `Cell` so the accessor takes `&self`.
    last_regime: Cell<Option<PaintRegimeTag>>,
    /// `GridSignals` drained by the last `paint_if_dirty`. Empty before
    /// the first paint. `Cell` so the accessor takes `&self`.
    last_signals: Cell<GridSignals>,
}

impl<S, M> Orchestrator<S, M>
where
    S: Surface,
    S::P: BlitPainter,
    M: CanvasModel,
{
    pub fn new(grid_surface: S, overlay_surface: S) -> Self {
        let grid_renderer = GridRenderer::for_layer(grid_surface.clone_painter());
        let overlay_renderer = OverlayRenderer::for_layer(overlay_surface.clone_painter());
        Self {
            grid: LayerBase::new(grid_surface, grid_renderer),
            overlay: LayerBase::new(overlay_surface, overlay_renderer),
            theme: CanvasTheme::light(),
            selection: SelectionLayer::default(),
            autofill: AutofillLayer::default(),
            clipboard: ClipboardLayer::default(),
            point_mode: PointModeLayer::default(),
            formula_refs: FormulaRefsLayer::default(),
            model: None,
            last_frame: None,
            size: CanvasSize { w: 0.0, h: 0.0 },
            pending_content: PaneRegionMask::EMPTY,
            last_regime: Cell::new(None),
            last_signals: Cell::new(GridSignals::empty()),
        }
    }

    /// Regime stamped by the last `paint_if_dirty`. `None` before the
    /// first paint. Read by the recording pipeline.
    pub fn last_regime(&self) -> Option<PaintRegimeTag> {
        self.last_regime.get()
    }

    /// `GridSignals` word the last `paint_if_dirty` acted upon. Empty
    /// before the first paint.
    pub fn last_signals(&self) -> GridSignals {
        self.last_signals.get()
    }

    /// Resize both layers in one call. No public per-layer resize, so
    /// callers can't leave the pair half-sized.
    pub fn resize(&mut self, size: CanvasSize, dpr: i32) {
        self.size = size;
        self.grid.resize(size, dpr);
        self.overlay.resize(size, dpr);
    }

    /// Conservative repaint blanket. Drops `last_frame` so the next
    /// `paint_if_dirty` falls to `Fresh` — the cheaper `SlotsReuse` /
    /// `Viewport` arms gate on `last_frame.is_some()`. Raises
    /// `STRUCTURAL | OVERLAY` — explicitly *not* `CONTENT`, which is
    /// reserved for real cell-value changes via `mark_content_dirty`.
    pub fn request_repaint(&mut self) {
        self.last_frame = None;
        self.pending_content = PaneRegionMask::EMPTY;
        self.grid
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// Bulk-push every overlay primitive in one comparison. The per-field
    /// setters each raise OVERLAY independently; folding them into one
    /// pass lets the Leptos host's per-frame reactive memo cost a single
    /// raise instead of four.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        let RenderOverlays {
            extend_to,
            clipboard,
            point_range,
            formula_refs,
            active_ref: _,
        } = overlays;
        let changed = self.autofill.extend_to != extend_to
            || self.clipboard.clipboard != clipboard
            || self.point_mode.point_range != point_range
            || self.formula_refs.refs != formula_refs;
        if !changed {
            return;
        }
        self.autofill.extend_to = extend_to;
        self.clipboard.clipboard = clipboard;
        self.point_mode.point_range = point_range;
        self.formula_refs.refs = formula_refs;
        self.overlay.raise(GridSignals::OVERLAY);
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        if self.autofill.extend_to != target {
            self.autofill.extend_to = target;
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        if self.clipboard.clipboard != area {
            self.clipboard.clipboard = area;
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        if self.point_mode.point_range != range {
            self.point_mode.point_range = range;
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        if self.formula_refs.refs != refs {
            self.formula_refs.refs = refs;
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    /// Push a theme. Value-compares against `self.theme` and, on change,
    /// drops `last_frame`, invalidates the renderer paint cache, and
    /// marks both layers dirty. Theme is frame-wide — the per-cell pixel
    /// cache holds the old palette and `is_still_valid` does not check
    /// theme, so without the cache invalidation `SlotsReuse` would
    /// repaint stale-color cells under fresh chrome.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != self.theme {
            self.theme = theme;
            self.last_frame = None;
            self.grid.invalidate_paint_cache();
            self.grid
                .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
            self.overlay
                .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
        }
    }

    pub fn set_theme_variables(&mut self, vars: ThemeVariables) {
        self.set_theme(vars.build());
    }

    /// Push a new data model. The generic-`M` API loses the Rc::ptr_eq
    /// idempotence today's IronCanvas had: every set_model is treated as
    /// a change. JS-side typically pushes once per workbook, so the
    /// difference is the worst-case repaint after a redundant push.
    pub fn set_model(&mut self, model: M) {
        self.model = Some(model);
        // `is_still_valid` doesn't see model identity, so a workbook swap
        // with the same scroll/sheet/freeze/size would otherwise reuse the
        // prev pane_set (stale row heights / column widths).
        self.last_frame = None;
        self.pending_content = PaneRegionMask::EMPTY;
        self.grid
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// Mark the overlay dirty. Selection, autofill, formula-ref, and
    /// clipboard signals funnel through here; grid escalation on scroll /
    /// freeze / sheet / size change is owned by `paint_if_dirty` via
    /// `is_still_valid`, not duplicated at the callsite.
    pub fn request_overlay_repaint(&mut self) {
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// Typed cell-content-changed signal. Marks the named panes' cached
    /// buffers stale so the next `paint_if_dirty` refetches their values
    /// from the model via the `SlotsReuse` arm (mask = these panes) —
    /// fixes the recalc bug where a formula dependent on an edited
    /// cell silently kept painting the stale cached value.
    pub fn mark_content_dirty(&mut self, mask: PaneRegionMask) {
        self.pending_content |= mask;
        self.grid.raise(GridSignals::CONTENT);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.size
    }

    pub fn theme(&self) -> &CanvasTheme {
        &self.theme
    }

    pub fn selection(&self) -> &SelectionLayer {
        &self.selection
    }

    /// Cross-crate test surface — the recorder backend reads the grid
    /// surface to inspect emitted `DrawOp`s. Production must not branch
    /// on this.
    #[doc(hidden)]
    pub fn grid_surface(&self) -> &S {
        &self.grid.surface
    }

    #[doc(hidden)]
    pub fn overlay_surface(&self) -> &S {
        &self.overlay.surface
    }

    // Query API. All queries resolve against `last_frame`, the snapshot
    // emitted by the most recent `paint_if_dirty`. Before the first paint
    // `last_frame` is `None` and every query returns its absent variant.

    pub fn hit_test(&self, x: f64, y: f64) -> HitTest {
        let Some(frame) = self.last_frame.as_ref() else {
            return HitTest::Outside;
        };
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        let layers: [&dyn Layer; 5] = [
            &self.formula_refs,
            &self.point_mode,
            &self.clipboard,
            &self.autofill,
            &self.selection,
        ];
        // No live selection → pass a zero range; the decoration layers that
        // consult `sel` (autofill, formula-refs) treat it as "no anchor"
        // and naturally fall through to the frame's pure cell hit-test.
        let sel = self.selection.selection_range.unwrap_or_default();
        for layer in layers {
            if let Some(hit) = layer.hit_test(frame, sel, xi, yi) {
                return hit;
            }
        }
        frame.hit_test(xi, yi)
    }

    /// Resolve a pixel coordinate to a cell (row, column), bypassing every
    /// decoration layer. The layer-aware `hit_test` is the right tool for
    /// pointer events that start interactions (mousedown), but a drag
    /// already in flight needs the underlying cell *regardless* of which
    /// overlay rectangle the cursor happens to be over — otherwise an
    /// overlay (e.g. `FormulaRefsLayer`) shadows its own cell and the host
    /// can't read pointer motion that re-enters the overlay's bounds.
    /// Returns `None` before the first paint or when the cursor falls in
    /// chrome / off-grid.
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        let frame = self.last_frame.as_ref()?;
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        let row = frame.pane_set.pixel_to_row(yi)?;
        let col = frame.pane_set.pixel_to_col(xi)?;
        Some((row, col))
    }

    pub fn resize_handle_at(&self, x: f64, y: f64, tolerance: f64) -> Option<ResizeTarget> {
        self.last_frame.as_ref()?.resize_handle_at(
            x.round() as i32,
            y.round() as i32,
            tolerance.round() as i32,
        )
    }

    pub fn cell_rect(&self, row: i32, column: i32) -> Option<PixelRect> {
        self.last_frame.as_ref()?.cell_rect(row, column)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame
            .as_ref()?
            .autofill_handle(self.selection.selection_range?)
    }

    /// Paint whichever layers are dirty. Dispatches via `decide` into one
    /// of four named regimes: `Overlay`, `Viewport`, `SlotsReuse`, `Fresh`.
    /// The `match` is exhaustive — adding a regime breaks the build here
    /// by design.
    pub fn paint_if_dirty(&mut self) {
        let signals = self.grid.drain_signals() | self.overlay.drain_signals();
        if signals.is_empty() {
            return;
        }
        // Lift the model out so subsequent paint methods can take `&mut
        // self` without overlapping the model borrow. Restored at the end.
        let Some(model) = self.model.take() else {
            return;
        };

        {
            let model_dyn: &dyn CanvasModel = &model;
            let regime = self.decide(signals, model_dyn);
            self.last_regime.set(Some(PaintRegimeTag::from(&regime)));
            self.last_signals.set(signals);
            match regime {
                PaintRegime::Overlay => self.paint_overlay_regime(model_dyn),
                PaintRegime::Viewport(plan) => self.paint_viewport_regime(model_dyn, plan),
                PaintRegime::SlotsReuse { mask, signals } => {
                    self.paint_slots_reuse_regime(model_dyn, mask, signals)
                }
                PaintRegime::Fresh(signals) => self.paint_fresh_regime(model_dyn, signals),
            }
            self.pending_content = PaneRegionMask::EMPTY;
        }

        self.model = Some(model);
    }

    /// Classify which paint regime to run for the current state. Pure over
    /// `&self`; arm methods own the mutation. The signal bits are consumed
    /// by the caller via `drain_signals`, so we take them as a parameter
    /// rather than re-reading the take-and-clear gate.
    fn decide(&self, sig: GridSignals, model: &dyn CanvasModel) -> PaintRegime {
        let content_dirty = sig.contains(GridSignals::CONTENT);
        let validity = self
            .last_frame
            .as_ref()
            .map_or(FrameValidity::Rebuild, |f| {
                f.is_still_valid(model, self.size)
            });

        if !sig.grid_dirty()
            && sig.overlay_dirty()
            && matches!(validity, FrameValidity::SlotsReuse)
            && self.last_frame.is_some()
        {
            return PaintRegime::Overlay;
        }

        // Blit detection is geometric: `screen_for_blit` diffs `last_frame`'s
        // scroll/freeze/sheet/size against the model and returns a plan
        // only on a real viewport shift. Gated on CONTENT (a blit on
        // stale content propagates wrong pixels — the recalc bug) but
        // not on a typed VIEWPORT signal: no JS-facing setter raises
        // VIEWPORT today, so requiring it would dead-code this arm.
        // Blit needs the previous frame's active-cell snapshot to re-hash
        // against live state and reject the fast-path on a content change.
        // Without a live selection there is nothing to re-hash, so the
        // blit attempt is skipped entirely and dispatch falls through.
        if !content_dirty {
            if let Some(active) = self.selection.active_cell.as_ref() {
                if let Some(plan) = self
                    .last_frame
                    .as_ref()
                    .and_then(|f| f.screen_for_blit(model, self.size, &self.theme, active))
                {
                    return PaintRegime::Viewport(plan);
                }
            }
        }

        if matches!(validity, FrameValidity::SlotsReuse) && self.last_frame.is_some() {
            let mask = if content_dirty && !self.pending_content.is_empty() {
                self.pending_content
            } else {
                PaneRegionMask::ALL
            };
            return PaintRegime::SlotsReuse { mask, signals: sig };
        }

        PaintRegime::Fresh(sig)
    }

    /// Refresh per-paint overlay state from the live model and mirror
    /// the selection rectangle into `AutofillLayer` so the preview is
    /// paint-coherent with the painted selection. Single source of
    /// truth for the cross-decoration mirror — every regime calls this
    /// instead of `self.selection.refresh(model)` directly.
    fn refresh_overlay_state(&mut self, model: &dyn CanvasModel) {
        self.selection.refresh(model);
        self.autofill.selection_range = self.selection.selection_range.unwrap_or_default();
    }

    /// Overlay-only fast path. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. `decide` proves the
    /// preconditions (slot vecs still match, `last_frame` is `Some`).
    fn paint_overlay_regime(&mut self, model: &dyn CanvasModel) {
        self.refresh_overlay_state(model);
        let Some(prev) = self.last_frame.as_ref() else {
            return;
        };
        self.overlay.paint_overlay_layer(
            model,
            prev,
            &self.selection,
            &[
                &self.autofill,
                &self.clipboard,
                &self.point_mode,
                &self.formula_refs,
            ],
        );
    }

    /// Scroll-blit fast path. `decide` already filtered no-op scrolls and
    /// viewport shifts where the kept band can't be reused; we trust the
    /// verdict and the supplied plan. Always repaints the overlay too —
    /// a viewport shift moves every overlay primitive's pixel position.
    ///
    /// `Chrome::next(Blit)` may demote to `Fresh` when in-place reuse
    /// rejects (e.g. row-header digit boundary). We dispatch on
    /// `frame.kind` so the demoted path takes the full repaint with cache
    /// invalidation, instead of a `paint_grid_blit` that would carry
    /// stale per-pane caches against the freshly rebuilt slot vecs.
    fn paint_viewport_regime(&mut self, model: &dyn CanvasModel, plan: BlitPlan) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        let frame = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::Blit(&plan),
        );
        match frame.kind {
            FrameKindTag::Blitted => self.grid.paint_grid_blit(model, &frame, &plan),
            FrameKindTag::Fresh => {
                self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
                self.grid.invalidate_paint_cache();
                self.grid.paint_grid(model, &frame);
            }
            FrameKindTag::SlotsReused => unreachable!(
                "Chrome::next(Blit) only returns Blitted (reuse) or Fresh (fallback)"
            ),
        }
        self.refresh_overlay_state(model);
        self.overlay.paint_overlay_layer(
            model,
            &frame,
            &self.selection,
            &[
                &self.autofill,
                &self.clipboard,
                &self.point_mode,
                &self.formula_refs,
            ],
        );
        self.last_frame = Some(frame);
    }

    /// SlotsReuse regime: prev's slot vecs survive (viewport unchanged);
    /// only `pane_cache` entries inside `mask` are invalidated so
    /// `render_pane` refetches there. Unmasked panes fingerprint-skip.
    fn paint_slots_reuse_regime(
        &mut self,
        model: &dyn CanvasModel,
        mask: PaneRegionMask,
        signals: GridSignals,
    ) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        let frame = Chrome::next(
            Some(prev),
            model,
            self.size,
            &self.theme,
            FramePath::SlotsReuse,
        );

        self.grid.invalidate_pane_cache(mask);
        self.grid.invalidate_paint_cache();

        self.grid.paint_grid(model, &frame);
        // Refresh the selection snapshot unconditionally: even on a
        // CONTENT-only signal the grid just repainted with new values,
        // so the next paint's `screen_for_blit` must compare against
        // the post-edit hash.
        self.refresh_overlay_state(model);
        if signals.overlay_dirty() {
            self.overlay.paint_overlay_layer(
                model,
                &frame,
                &self.selection,
                &[
                    &self.autofill,
                    &self.clipboard,
                    &self.point_mode,
                    &self.formula_refs,
                ],
            );
        }
        self.last_frame = Some(frame);
    }

    /// Full grid repaint. Slot vecs walked fresh from the model; the new
    /// vecs make any cross-frame fingerprint compare meaningless, so every
    /// pane repaints. Selected when slot vecs diverged or no prior frame.
    /// The `CONTENT` bit gates `PaneCache` invalidation: a content edit
    /// escalated to Fresh (e.g. via concurrent scroll) means the cache's
    /// range-matched buffers may now be stale against the new slot vecs.
    fn paint_fresh_regime(&mut self, model: &dyn CanvasModel, signals: GridSignals) {
        let prev = self.last_frame.take();
        let frame = Chrome::next(prev, model, self.size, &self.theme, FramePath::Fresh);

        if signals.contains(GridSignals::CONTENT) {
            self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
        }
        self.grid.invalidate_paint_cache();
        self.grid.paint_grid(model, &frame);
        self.refresh_overlay_state(model);
        if signals.overlay_dirty() {
            self.overlay.paint_overlay_layer(
                model,
                &frame,
                &self.selection,
                &[
                    &self.autofill,
                    &self.clipboard,
                    &self.point_mode,
                    &self.formula_refs,
                ],
            );
        }
        self.last_frame = Some(frame);
    }
}
