//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! `Orchestrator<S>` (in [`crate::orchestrator`]) owns two
//! [`LayerBase<S, R>`](crate::layer::LayerBase) values: one for the grid,
//! one for the overlay. Each `LayerBase` holds a [`Surface`](crate::layer::Surface),
//! a [`PaintGate`](crate::layer::PaintGate), and a layer-specific renderer
//! wrapping [`RendererCore`]. In the wasm build the surface is
//! `iron_canvas_canvas2d::WebSurface`; the grid context uses `alpha: false`
//! (opaque, skips alpha compositing) and the overlay uses
//! `alpha: true, desynchronized: true`. The renderer is long-lived per
//! layer, so the painter's cached fill/stroke/font/line-width state
//! survives across frames.
//!
//! State pushes from the host mark layers dirty. `Orchestrator::paint_if_dirty`
//! drives each dirty layer through its `LayerBase` paint method:
//! `paint_grid` / `paint_grid_blit` for the grid, `paint_overlay_layer`
//! for the overlay. The grid path calls into [`RendererCore::render_grid`];
//! the overlay path iterates the [`Layer`](crate::decoration::Layer)
//! decorations in `crate::decoration` and calls back into `RendererCore`
//! for the active-cell repaint and header highlights.
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `paint_if_dirty` per dirty layer:
//!
//! - [`RendererCore::render_grid`] paints cells (four frozen-pane
//!   quadrants, each running five cell sub-passes: bg, then CF decoration,
//!   then grid borders, then explicit borders, then text), then frozen
//!   separators, then headers, then the corner box.
//! - `LayerBase::paint_overlay_layer` orchestrates the decorations in
//!   `crate::decoration` (selection, autofill, clipboard, point-mode,
//!   formula-refs) plus header highlights.
//!
//! The cell sub-pass order is the contract: grid borders run across the
//! whole pane before explicit borders, so an explicit `right` on cell A
//! wins over cell B's grid `left` at the shared pixel column. Text runs
//! last so overflow is never clipped by a neighbour's bg.
//!
//! # Frozen panes
//!
//! The grid splits into up to four quadrants (`TopLeft`, `TopRight`,
//! `BottomLeft`, `BottomRight`) based on frozen rows and columns. Each
//! quadrant is rendered by `render_pane()` against a different
//! [`PaneRegion`](crate::chrome::PaneRegion); a thick separator line
//! marks the freeze boundary:
//!
//! ```text
//!           frozen cols │ scrollable cols
//!         ──────────────┼──────────────────
//! frozen   TopLeft      │ TopRight
//! rows     (static)     │ (scrolls in X)
//!         ──────────────┼──────────────────
//! scroll   BottomLeft   │ BottomRight
//! rows     (scrolls Y)  │ (scrolls in X and Y)
//! ```
//!
//! With no frozen rows or columns the grid is a single `BottomRight`
//! quadrant.

pub mod blit_work;
pub mod cache;
pub mod cell;
pub mod cf_types;
pub mod frame;
// `renderer/overlay/` has moved to `src/decoration/`. Each decoration is
// a struct that impls `Layer`; the orchestration that used to live in
// `RendererCore::render_overlays` is now in
// `LayerBase::paint_overlay_layer` (src/layer/mod.rs).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::CanvasModel;
pub use crate::chrome::PaneRegion;
use crate::chrome::{BlitPlan, Chrome, PaneRegionMask};
use crate::geometry::prim::Axis;
use crate::renderer::blit_work::widen_blit_strip_to_pixel_clip;
use crate::renderer::cache::{FrameCache, PaneCache, PaneShiftPrep};
use crate::signal::RowSpan;
pub use cache::ColorIntern;
pub use cache::FontIntern;

pub use self::cell::text::{TextLine, layout_into};

use crate::orchestrator::{BlitFallback, FrameOutcome, FrameTrace, PaneVerdict};
use crate::painter::{BlitPainter, GroupClass, Painter};
use crate::style::{CellDecoration, CellKind, CellStyle};
use crate::types::coord::RCRange;
use crate::types::fetched::Fetched;

/// Per-pane staging for the blit preflight, indexed by `PaneRegion as usize`
/// (mirroring `PaneCache`'s own `[PaneBuffers; 4]`).
/// [`RendererCore::prefetch_blit_strips`] fetches and bridge-validates each
/// shifted pane's revealed strip into one of these BEFORE any pixel is
/// shifted; `render_pane_blit` then paints from the staged buffers instead of
/// re-fetching. `Cell`-wrapped so the take/park rhythm keeps the `Vec`
/// capacity warm across frames — never a per-frame allocation.
#[derive(Default)]
struct BlitStripStage {
    /// True only between a successful preflight fetch and the paint that
    /// consumes it; `render_pane_blit` clears it via `take`.
    ready: Cell<bool>,
    strip: Cell<Option<RCRange>>,
    /// Set when the preflight validated a pane's WHOLE range rather than a
    /// strip (`unshiftable_pane_is_safe`, for a pane the blit could not shift).
    /// The range it was fetched for, so the `render_pane` that follows can
    /// adopt the fetch instead of repeating it — that pane is otherwise the
    /// only place in the renderer that crosses the bridge twice for the same
    /// cells in one frame. Cleared by whoever consumes it.
    full_pane: Cell<Option<RCRange>>,
    styles: Cell<Vec<Fetched<CellStyle>>>,
    values: Cell<Vec<Fetched<String>>>,
    cell_types: Cell<Vec<Fetched<CellKind>>>,
    decorations: Cell<Vec<Fetched<CellDecoration>>>,
}

/// Shared renderer core. Holds the painter `P`, dpr, the per-frame
/// `FrameCache`, and the renderer-lifetime intern tables (font, column
/// labels, per-cell color overrides). The two layer wrappers
/// (`GridRenderer`, `OverlayRenderer`) each own a `RendererCore` and
/// re-export only what their layer is allowed to perform: `GridRenderer`
/// exposes `render_grid` + the four-phase pipeline; `OverlayRenderer`
/// exposes `painter()` + `repaint_active_cell` + `render_header_highlights`
/// for `LayerBase::paint_overlay_layer` to drive the decoration walk.
pub struct RendererCore<P: Painter> {
    /// The surface owns the painter as the semantic source of truth; the
    /// renderer holds a shared handle so paint methods reach the painter
    /// without re-borrowing through the surface on every call.
    pub painter: Rc<P>,
    dpr: f64,
    pub frame_cache: FrameCache,
    /// Renderer-lifetime per-pane bulk-fetch buffers + last-fetched range.
    /// Sibling of the intern tables below; survives across frames so
    /// `render_pane` can short-circuit when a pane's address didn't
    /// change (Stage 3.2) or strip-fetch the new band (Stage 3.3).
    pub pane_cache: PaneCache,
    /// Renderer-lifetime intern table for `ctx.font` strings. Lives outside
    /// `FrameCache` because identical fonts repeat across frames, not just
    /// within a single paint.
    pub font_intern: FontIntern,
    /// Renderer-lifetime intern of per-cell color overrides (border + text).
    /// Hot-path callers (`BorderPaint::resolve`, `CellTextStyle::resolve`)
    /// previously allocated a fresh `String` per cell per frame; the intern
    /// makes those calls `Rc::clone` after the first sighting of each color.
    pub color_intern: ColorIntern,
    /// Per-pane blit-preflight staging (`prefetch_blit_strips` fills it,
    /// `render_pane_blit` drains it). Renderer-lifetime scratch, not part of
    /// the pane cache — mutating it never counts as touching cache state.
    blit_stage: [BlitStripStage; 4],
    /// This frame's paint attribution. `Cell` because every paint method runs
    /// on `&self` (the crate's paint-never-holds-`&mut` convention), and
    /// `FrameTrace` is `Copy`.
    trace: Cell<FrameTrace>,
}

impl<P: Painter> RendererCore<P> {
    pub fn painter(&self) -> &P {
        self.painter.as_ref()
    }

    /// Clear the trace for a new frame. Called once by `paint_if_dirty`
    /// before dispatch, never by a paint method — a paint method that reset
    /// it would erase the sibling panes' verdicts.
    pub fn reset_trace(&self) {
        self.trace.set(FrameTrace::default());
    }

    pub fn trace(&self) -> FrameTrace {
        self.trace.get()
    }

    fn trace_pane(&self, pane: PaneRegion, verdict: PaneVerdict) {
        let mut t = self.trace.get();
        if let Some(slot) = t.panes.get_mut(pane as usize) {
            *slot = Some(verdict);
        }
        self.trace.set(t);
    }

    /// Record that a `Viewport` frame lost the strip path for `pane`. Only the
    /// first such pane is kept — it is already enough to explain the frame's
    /// cost, and the fix is per-reason, not per-pane.
    fn trace_blit_fallback(&self, pane: PaneRegion, cold_cache: bool) {
        let mut t = self.trace.get();
        if t.blit_fallback.is_none() {
            t.blit_fallback = Some(BlitFallback { pane, cold_cache });
        }
        self.trace.set(t);
    }

    fn trace_frame_held(&self, pane: PaneRegion) {
        let mut t = self.trace.get();
        t.outcome = FrameOutcome::HeldOnBridgeFailure(pane);
        self.trace.set(t);
    }

    /// Charge one bulk-fetch round (all four accessors) over `range`. The
    /// direct read on invariant I1: the model round-trip is unconditional, so
    /// this rises with pane area no matter which verdict follows.
    fn trace_fetch(&self, range: RCRange) {
        let cells = range.height() as usize * range.width() as usize;
        let mut t = self.trace.get();
        t.fetched_cell_slots += cells * 4;
        self.trace.set(t);
    }
}

impl<P: Painter> RendererCore<P> {
    /// Wipe the per-frame paint state and restore the sticky text defaults
    /// the renderer assumes at every entry point. Routed through the
    /// `Painter` trait so any backend (Canvas-2D today, Recorder/SVG later)
    /// gets the same reset semantics.
    pub fn invalidate_paint_cache(&mut self) {
        self.painter.invalidate_cache();
        self.painter.reset_text_defaults();
    }

    /// React to a backing-store resize: push the new DPR through the
    /// painter's transform, store it for snap math, and clear caches.
    pub fn resize_for_dpr(&mut self, dpr: f64) {
        self.painter.apply_dpr_transform(dpr);
        self.dpr = dpr;
        self.invalidate_paint_cache();
    }

    /// Layer-friendly constructor: caller owns canvas sizing + DPR scaling.
    /// Canvas size and theme both live on the per-frame `Chrome`,
    /// not on the renderer. Takes the painter as an `Rc` so the surface that
    /// owns the painter can hand the renderer its own owning handle.
    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            painter,
            dpr: 1.0,
            frame_cache: FrameCache {
                text_slots: Cell::new(Vec::new()),
                show_grid: Cell::new(true),
                text_lines: Cell::new(Vec::new()),
                wrap_buf: RefCell::new(String::new()),
                strip_styles: Cell::new(Vec::new()),
                strip_values: Cell::new(Vec::new()),
                strip_cell_types: Cell::new(Vec::new()),
                strip_decorations: Cell::new(Vec::new()),
            },
            pane_cache: PaneCache::default(),
            font_intern: FontIntern::new(),
            color_intern: ColorIntern::new(),
            blit_stage: std::array::from_fn(|_| BlitStripStage::default()),
            trace: Cell::new(FrameTrace::default()),
        }
    }

    /// Paint the grid layer for a fresh / slots-reuse frame: cells (per
    /// quadrant), frozen separators, both header strips, corner box. Does
    /// **not** clear the canvas — caller owns the clear so layer-owned
    /// renderers can paint a background fill instead.
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model);

        // `frame.stale_panes` is `ALL` on Fresh; narrower on SlotsReuse —
        // either way each region listed needs its 5-pass walk.
        self.painter.begin_group(GroupClass::Cells);
        for pane in frame.stale_panes.regions() {
            self.render_pane(model, pane, frame);
        }
        self.painter.end_group();

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        self.painter.begin_group(GroupClass::Headers);
        if frame.row_header_thickness > 0 {
            self.render_headers_base(Axis::Row, frame);
        }
        if frame.col_header_thickness > 0 {
            self.render_headers_base(Axis::Column, frame);
        }
        self.painter.end_group();

        self.draw_corner_box_if_needed(frame);

        self.painter.end_group();
    }

    /// Scroll-blit variant: caller's `Painter::blit` already shifted the
    /// kept band. We prepare each shifted pane's cache (`prepare_shift`
    /// rotates the buffers in place), then dispatch ONCE on the typed
    /// [`PaneShiftPrep`]: a `Shifted` pane strip-paints (BottomRight wrapped
    /// in a clip to `plan.repaint_strip`); every other outcome falls through
    /// to a full `render_pane` repaint. Only the scroll-axis header strip is
    /// refreshed (the cross-axis header is unchanged).
    ///
    /// On a blit frame `frame.stale_panes == plan.shift_panes()`
    /// (`next_blit` seeds `stale_panes` from `shift_panes`), so one loop over
    /// the stale panes covers exactly the shifted set.
    pub fn render_grid_blit(&self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model);

        self.painter.begin_group(GroupClass::Cells);
        for pane in frame.stale_panes.regions() {
            let Some(new_range) = pane.range(frame) else {
                // Never-cached / empty live range: nothing to shift, full fetch.
                self.pane_cache.pane(pane).range.set(None);
                self.render_pane(model, pane, frame);
                continue;
            };
            // `prepare_shift` rotates the cache buffers and reports why; the
            // dispatch decision is made here, once, from the typed result.
            match self
                .pane_cache
                .pane(pane)
                .prepare_shift(new_range, plan.axis)
            {
                PaneShiftPrep::Shifted {
                    prev_range,
                    new_range,
                } => {
                    // Build the pane's blit work in two halves: the cache emits
                    // address-space work (no `Chrome` dependency), then a
                    // renderer-local helper widens it against this frame's slot
                    // geometry and attaches the pixel clip.
                    let Some(address_work) = self
                        .pane_cache
                        .plan_blit_pane(prev_range, new_range, plan.axis)
                    else {
                        self.render_pane(model, pane, frame);
                        continue;
                    };
                    let work = widen_blit_strip_to_pixel_clip(frame, plan, pane, address_work);
                    match work.pixel_clip {
                        Some(clip) => {
                            self.painter.push_clip(clip);
                            self.render_pane_blit(model, frame, &work);
                            self.painter.pop_clip();
                        }
                        None => self.render_pane_blit(model, frame, &work),
                    }
                }
                PaneShiftPrep::MissingCache | PaneShiftPrep::IncompatibleRange { .. } => {
                    self.render_pane(model, pane, frame);
                }
            }
        }
        self.painter.end_group();

        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        // Only the scroll-axis header strip shifted; the cross-axis
        // strip's pixels are unchanged.
        self.painter.begin_group(GroupClass::Headers);
        let axis_thickness = match plan.axis {
            Axis::Row => frame.row_header_thickness,
            Axis::Column => frame.col_header_thickness,
        };
        if axis_thickness > 0 {
            self.render_headers_base(plan.axis, frame);
        }
        self.painter.end_group();

        self.draw_corner_box_if_needed(frame);

        self.painter.end_group();
    }

    /// Damage variant: prior pixels stay; only the damaged full-width row
    /// bands per pane refetch + repaint. Same outer sequence as
    /// `render_grid` — cells, then frozen separators (they must win pixels
    /// back from the band's re-stroked grid lines at the freeze boundary),
    /// then headers and corner, all inside the Grid group.
    pub fn render_grid_damage(&self, model: &dyn CanvasModel, frame: &Chrome, spans: &[RowSpan]) {
        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model);

        self.painter.begin_group(GroupClass::Cells);
        for pane in PaneRegionMask::ALL.regions() {
            self.render_pane_damage(model, frame, pane, spans);
        }
        self.painter.end_group();

        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        self.painter.begin_group(GroupClass::Headers);
        if frame.row_header_thickness > 0 {
            self.render_headers_base(Axis::Row, frame);
        }
        if frame.col_header_thickness > 0 {
            self.render_headers_base(Axis::Column, frame);
        }
        self.painter.end_group();

        self.draw_corner_box_if_needed(frame);

        self.painter.end_group();
    }

    /// Paint the header corner box, gated for *correctness*: at thickness 0 it
    /// would still stroke 0.5px border lines spanning the full canvas. Shared
    /// by `render_grid` and `render_grid_blit` (the one block identical between
    /// them — the header strips differ, full vs scroll-axis only).
    fn draw_corner_box_if_needed(&self, frame: &Chrome) {
        if frame.row_header_thickness > 0 && frame.col_header_thickness > 0 {
            self.painter.begin_group(GroupClass::Corner);
            self.draw_corner_box(frame);
            self.painter.end_group();
        }
    }

    /// Cache the per-sheet grid-line toggle once for this frame so the
    /// hot per-cell `paint_borders_grid` walk doesn't re-enter the model.
    /// Falls back to "show" on model failure, matching Excel's default-on.
    fn cache_show_grid(&self, model: &dyn CanvasModel) {
        let sheet = model.get_selected_sheet();
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));
    }
}

// Layer-facing wrappers
//
// `GridRenderer` and `OverlayRenderer` each own a `RendererCore` and re-export
// only the operations their layer is allowed to perform. `LayerOps` is the
// paint-backend-agnostic subset (just `resize_for_dpr`); the Canvas-2D
// passthroughs (`ctx_ref` for the layer's own clear/fill, `invalidate_paint_cache`)
// live as inherent methods on the `<CanvasPainter>` impl so a future SvgPainter
// can satisfy `LayerOps` without `web_sys`.

/// Backend-agnostic resize hook. Called by `LayerBase::resize` whenever the
/// backing store's DPR changes; everything else stays on the wrapper's
/// inherent surface. `Painter` ties the renderer's painter type to the
/// `LayerBase`'s `Surface::P` at the type level.
pub trait LayerOps {
    type Painter: Painter;
    fn resize_for_dpr(&mut self, dpr: f64);
}

pub struct GridRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> GridRenderer<P> {
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) {
        self.core.render_grid(model, frame);
    }

    pub fn render_grid_blit(&self, model: &dyn CanvasModel, frame: &Chrome, plan: &BlitPlan) {
        self.core.render_grid_blit(model, frame, plan);
    }

    /// Whole-frame blit preflight — see [`RendererCore::prefetch_blit_strips`].
    /// `paint_grid_blit` calls this BEFORE shifting any pixel; a `false`
    /// return means the frame is a no-op.
    pub fn prefetch_blit_strips(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        self.core.prefetch_blit_strips(model, frame, plan)
    }

    pub fn render_grid_damage(&self, model: &dyn CanvasModel, frame: &Chrome, spans: &[RowSpan]) {
        self.core.render_grid_damage(model, frame, spans);
    }

    /// Drop cached pane-buffer ranges for the masked panes. Plumbed through
    /// from the orchestrator's content-dirty regime arms so a
    /// cell-content-changed paint can force the named panes to refetch on
    /// their next `render_pane` while unmasked panes keep their
    /// fingerprint-skip win.
    pub fn invalidate_pane_cache(&self, mask: crate::chrome::PaneRegionMask) {
        self.core.pane_cache.invalidate(mask);
    }

    pub fn reset_trace(&self) {
        self.core.reset_trace();
    }

    pub fn trace(&self) -> FrameTrace {
        self.core.trace()
    }

    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn invalidate_paint_cache(&mut self) {
        self.core.invalidate_paint_cache();
    }
}

impl<P: BlitPainter> GridRenderer<P> {
    pub fn painter_blit(
        &self,
        src: crate::geometry::pixel_rect::PixelRect,
        dst: crate::geometry::pixel_rect::PixelRect,
    ) {
        self.core.painter().blit(src, dst);
    }
}

impl<P: Painter> LayerOps for GridRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: f64) {
        self.core.resize_for_dpr(dpr);
    }
}

pub struct OverlayRenderer<P: Painter> {
    core: RendererCore<P>,
}

impl<P: Painter> OverlayRenderer<P> {
    pub fn for_layer(painter: Rc<P>) -> Self {
        Self {
            core: RendererCore::for_layer(painter),
        }
    }

    pub fn painter(&self) -> &P {
        self.core.painter()
    }

    pub fn render_header_highlights(
        &self,
        axis: crate::geometry::prim::Axis,
        frame: &Chrome,
        selection_range: crate::types::coord::RCRange,
    ) {
        self.core
            .render_header_highlights(axis, frame, selection_range);
    }

    pub fn repaint_active_cell(
        &self,
        model: &dyn CanvasModel,
        row: i32,
        column: i32,
        frame: &Chrome,
    ) {
        self.core.repaint_active_cell(model, row, column, frame);
    }
}

impl<P: Painter> LayerOps for OverlayRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: f64) {
        self.core.resize_for_dpr(dpr);
    }
}
