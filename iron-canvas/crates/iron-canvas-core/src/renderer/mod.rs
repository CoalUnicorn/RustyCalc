//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! `Orchestrator<S>` (in [`crate::orchestrator`]) owns two
//! [`LayerBase<S, R>`](crate::layer::LayerBase) values: one for the grid,
//! one for the overlay. Each `LayerBase` holds a [`Surface`](crate::layer::Surface)
//! and a layer-specific renderer wrapping [`RendererCore`] — and no dirty
//! state of its own. In the wasm build the surface is
//! `iron_canvas_canvas2d::WebSurface`; the grid context uses `alpha: false`
//! (opaque, skips alpha compositing) and the overlay uses
//! `alpha: true, desynchronized: true`. The renderer is long-lived per
//! layer, so the painter's cached fill/stroke/font/line-width state
//! survives across frames.
//!
//! State pushes from the host mark work on `Orchestrator`'s single pending
//! value. `Orchestrator::paint_if_dirty` picks a regime from it and drives
//! the layers that regime paints through their `LayerBase` paint method:
//! `paint_grid` / `paint_grid_blit` for the grid, `paint_overlay_layer`
//! for the overlay. The grid path calls into [`RendererCore::render_grid`];
//! the overlay path iterates the [`Layer`](crate::decoration::Layer)
//! decorations in `crate::decoration` and calls back into `RendererCore`
//! for the active-cell repaint and header highlights.
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `paint_if_dirty` per regime:
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
pub mod prepared;
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
use crate::pending_work::RowSpan;
use crate::renderer::cache::{FrameCache, PaneCache};
pub use cache::ColorIntern;
pub use cache::FontIntern;

pub use self::cell::text::{TextLine, layout_into};

use crate::orchestrator::{BlitFallback, FrameOutcome, FrameTrace, PaneVerdict};
use crate::painter::{BlitPainter, GroupClass, Painter};
pub(crate) use crate::renderer::prepared::PreparedCacheCommit;
use crate::renderer::prepared::{FetchedCells, PaneCacheAction, PaneCacheCommit, PreparedPane};
use crate::types::coord::RCRange;

pub(super) enum PaneExecution {
    Held,
    Untouched,
    Committed(PaneCacheCommit),
}

pub(crate) struct PreparedGridPaint {
    pub(crate) held: PaneRegionMask,
    pub(crate) cache_commit: PreparedCacheCommit,
}

/// Which header strips a grid execution repaints. A scroll blit shifts only
/// the scroll-axis strip's pixels, so repainting the cross-axis strip would
/// be work the frame never invalidated.
#[derive(Clone, Copy)]
enum GridHeaderScope {
    Both,
    Axis(Axis),
}

impl GridHeaderScope {
    fn paints(self, axis: Axis) -> bool {
        match (self, axis) {
            (Self::Both, Axis::Row)
            | (Self::Both, Axis::Column)
            | (Self::Axis(Axis::Row), Axis::Row)
            | (Self::Axis(Axis::Column), Axis::Column) => true,
            (Self::Axis(Axis::Row), Axis::Column) | (Self::Axis(Axis::Column), Axis::Row) => false,
        }
    }
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

    #[cfg(feature = "surface-introspection")]
    pub fn strip_scratch_capacities(&self) -> Vec<(usize, usize, usize, usize)> {
        self.frame_cache
            .strip_scratch
            .borrow()
            .iter()
            .map(FetchedCells::capacities)
            .collect()
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

    /// Charge one renderer bundle fetch over `range`. The legacy logical slot
    /// total remains a derived channel count for compatibility; the separate
    /// cell and batch counters make the trace useful without pretending to
    /// know how many host or adapter calls the model performed internally.
    fn trace_fetch(&self, range: RCRange) {
        let mut t = self.trace.get();
        t.fetched_cell_slots += FetchedCells::logical_channel_slots(range);
        t.fetched_cells += FetchedCells::addressed_cells(range);
        t.fetch_batches += 1;
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
                strip_scratch: RefCell::new(vec![FetchedCells::default()]),
            },
            pane_cache: PaneCache::default(),
            font_intern: FontIntern::new(),
            color_intern: ColorIntern::new(),
            trace: Cell::new(FrameTrace::default()),
        }
    }

    /// Paint the grid layer for a slots-reuse frame: cells (per quadrant),
    /// frozen separators, both header strips, corner box. Does **not**
    /// clear the canvas — caller owns the clear so layer-owned renderers
    /// can paint a background fill instead.
    ///
    /// Only ever called with a `SlotsReused`/`Blitted`-kind `frame`: the
    /// `Fresh` construction path routes through the separate
    /// [`Self::prepare_fresh_panes`]/[`Self::execute_fresh_grid`] pair
    /// instead (see their docs for why Fresh cannot share this method's
    /// tolerant-per-pane shape). `mask` is the pane scope the caller's
    /// `GridWork::Panes` planned. An explicit parameter rather than a
    /// `Chrome`-carried field, so consecutive calls against the same
    /// `Chrome` value can never leak one call's scope into the next.
    ///
    /// Returns the mask of panes whose content work was held (see
    /// `render_pane`) — `EMPTY` when every visited pane painted or skipped
    /// cleanly.
    pub fn render_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> PaneRegionMask {
        let result = self.execute_grid(model, frame, mask);
        let held = result.held;
        self.commit_pane_cache(result.cache_commit);
        held
    }

    /// The grid group sequence every strategy shares:
    /// `Grid -> Cells -> FrozenSep -> Headers -> Corner`. Only the work
    /// inside Cells differs between SlotsReuse, Damage, Fresh and Viewport,
    /// so `execute_cells` owns that and nothing else; its typed result
    /// passes straight back out.
    ///
    /// `execute_cells` runs on the infallible half of every strategy: a
    /// tolerant per-pane hold is folded into the callback's own result, never
    /// returned early through the shell, so the groups opened here always
    /// close.
    fn execute_grid_shell<T>(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        headers: GridHeaderScope,
        execute_cells: impl FnOnce() -> T,
    ) -> T {
        self.painter.begin_group(GroupClass::Grid);
        self.cache_show_grid(model, frame.sheet);

        self.painter.begin_group(GroupClass::Cells);
        let cells = execute_cells();
        self.painter.end_group();

        // Frozen separators paint AFTER cells so the thick divider wins
        // its pixels over the rightmost/bottommost frozen cell's grid stroke.
        self.painter.begin_group(GroupClass::FrozenSep);
        self.draw_frozen_separators(frame);
        self.painter.end_group();

        self.painter.begin_group(GroupClass::Headers);
        for axis in [Axis::Row, Axis::Column] {
            let thickness = match axis {
                Axis::Row => frame.row_header_thickness,
                Axis::Column => frame.col_header_thickness,
            };
            if headers.paints(axis) && thickness > 0 {
                self.render_headers_base(axis, frame);
            }
        }
        self.painter.end_group();

        self.draw_corner_box_if_needed(frame);

        self.painter.end_group();
        cells
    }

    pub(crate) fn execute_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> PreparedGridPaint {
        self.execute_grid_shell(model, frame, GridHeaderScope::Both, || {
            let mut held = PaneRegionMask::EMPTY;
            let mut cache_commit = PreparedCacheCommit::with_capacity(mask.regions().count());
            for pane in mask.regions() {
                match self.execute_pane(model, pane, frame) {
                    PaneExecution::Held => held = held.with(pane),
                    PaneExecution::Untouched => {}
                    PaneExecution::Committed(commit) => cache_commit.push(commit),
                }
            }
            PreparedGridPaint { held, cache_commit }
        })
    }

    /// Pure: prepare every pane in `mask` for a `Fresh`-kind frame — bulk
    /// fetch and bridge-check only, zero painter interaction of any kind
    /// (not even a group bracket). `None` on the first pane whose fetch
    /// reports a bridge failure, which is Fresh's whole-frame atomicity
    /// gate: `prepare_full_pane`'s own internal gate only fires when
    /// `frame.kind.reuses_slots()` (see that method's doc — a `Fresh`
    /// candidate has no prior committed pixels of its own to partially
    /// preserve, so its bridge-failure check is this call's job instead),
    /// so this loop re-checks `FetchedCells::has_bridge_failure` on every
    /// `Full` preparation directly.
    ///
    /// Zero painter interaction is load-bearing, not just a style choice:
    /// [`LayerBase::paint_grid_fresh`] must not clear the canvas, invalidate
    /// the paint cache, or open a single group on a held attempt — any of
    /// those would already be an observable op even though no pane pixel
    /// moved, which is exactly the atomicity a `Fresh` hold promises. That
    /// is why this method returns a prepared bundle for a *separate*
    /// execute step rather than pairing prepare-and-paint per pane the way
    /// `render_pane`'s tolerant walk does.
    ///
    /// `pub(crate)`, not `pub`: `PreparedPane` itself stays `pub(crate)`
    /// (an execution detail, not consumer API — see `renderer::prepared`'s
    /// module doc), so a `Vec<PreparedPane>` cannot cross the crate
    /// boundary. [`Self::render_grid_fresh`] is the `pub`, single-call
    /// entry point for a caller (including `tests/prepared_frame.rs`) that
    /// wants this pair's atomicity without the layer's bg-fill
    /// interleaving; [`crate::layer::LayerBase::paint_grid_fresh`] is the
    /// one caller that needs the two calls kept separate.
    pub(crate) fn prepare_fresh_panes(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> Option<Vec<PreparedPane>> {
        let mut prepared = Vec::with_capacity(4);
        for pane in mask.regions() {
            let Some(range) = pane.range(frame) else {
                prepared.push(PreparedPane::Empty {
                    pane,
                    cache_action: PaneCacheAction::Empty,
                });
                continue;
            };
            let Some(pane_prepared) = self.prepare_full_pane(model, pane, range, frame) else {
                self.recycle_prepared_panes(prepared);
                return None;
            };
            if let PreparedPane::Full { ref fetched, .. } = pane_prepared
                && fetched.has_bridge_failure()
            {
                self.trace_frame_held(pane);
                prepared.push(pane_prepared);
                self.recycle_prepared_panes(prepared);
                return None;
            }
            prepared.push(pane_prepared);
        }
        Some(prepared)
    }

    /// Infallible: paint every pane in `prepared` inside the shared
    /// [`Self::execute_grid_shell`] sequence — the execution half of the Fresh
    /// atomic path, reachable only once [`Self::prepare_fresh_panes`] has
    /// confirmed every pane clean. Sharing the shell is what keeps the op
    /// stream on a clean paint identical to `render_grid`'s; unlike
    /// `render_grid`, there is no held-pane loop in the callback at all — a
    /// bundle reaching this call is already known entirely healthy, matching
    /// [`Self::execute_blit`]'s prepare-then-execute shape (see
    /// `renderer::prepared`). The returned aggregate is installed only by the
    /// caller's completion boundary.
    ///
    /// `pub(crate)`, matching [`Self::prepare_fresh_panes`]'s own reasoning.
    pub(crate) fn execute_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: Vec<PreparedPane>,
    ) -> PreparedCacheCommit {
        self.execute_grid_shell(model, frame, GridHeaderScope::Both, || {
            let mut cache_commit = PreparedCacheCommit::with_capacity(prepared.len());
            for pane_prepared in prepared {
                let region = pane_prepared.region();
                let verdict = match &pane_prepared {
                    PreparedPane::Full { repaint, .. } => Some(PaneVerdict::from(&repaint.plan)),
                    PreparedPane::Empty { .. } => None,
                    PreparedPane::Damage { .. } | PreparedPane::Blit { .. } => {
                        unreachable!("prepare_fresh_panes only ever prepares Empty/Full panes")
                    }
                };
                let commit = match pane_prepared {
                    PreparedPane::Empty { pane, .. } => PaneCacheCommit::Empty { pane },
                    PreparedPane::Full { .. } => self.execute_full_pane(frame, pane_prepared),
                    PreparedPane::Damage { .. } | PreparedPane::Blit { .. } => {
                        unreachable!("prepare_fresh_panes only ever prepares Empty/Full panes")
                    }
                };
                if let Some(v) = verdict {
                    self.trace_pane(region, v);
                }
                cache_commit.push(commit);
            }
            cache_commit
        })
    }

    /// Combined prepare+execute for a `Fresh`-kind frame's cell/chrome
    /// pass, with no background fill of its own — mirrors
    /// [`Self::render_grid_blit`]'s single-call, `bool`-held shape rather
    /// than [`Self::render_grid`]'s `PaneRegionMask` one, since the atomic
    /// path has no partial-hold value to report (see
    /// [`Self::prepare_fresh_panes`]'s doc: it is always all-or-nothing).
    /// `pub`: the one entry point a caller outside `layer/mod.rs` — chiefly
    /// `tests/prepared_frame.rs`, exercising this exact atomicity at the
    /// same low level its `render_pane`/`render_pane_damage` sibling tests
    /// already use — can reach without naming the `pub(crate)`
    /// `PreparedPane` bundle that only lives between this method's two
    /// halves. [`crate::layer::LayerBase::paint_grid_fresh`] is the one
    /// caller that cannot use this directly: it needs the background fill
    /// injected between prepare succeeding and execution painting, which
    /// only the two-call form allows.
    ///
    /// Returns `true` (held) with zero painter interaction — not even a
    /// group bracket — on any pane's bridge failure; `false` once the
    /// frame actually painted.
    pub fn render_grid_fresh(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> bool {
        let Some(prepared) = self.prepare_fresh_panes(model, frame, mask) else {
            return true;
        };
        let cache_commit = self.execute_fresh_grid(model, frame, prepared);
        self.commit_pane_cache(cache_commit);
        false
    }

    /// Damage variant: prior pixels stay; only the damaged full-width row
    /// bands per pane refetch + repaint. The outer sequence is not restated
    /// here — it is the shared [`Self::execute_grid_shell`] `render_grid`
    /// also runs through, which is what guarantees the frozen separators
    /// still paint after the cells (winning their pixels back from the
    /// band's re-stroked grid lines at the freeze boundary).
    ///
    /// Returns the mask of panes whose damage work was held — see
    /// `render_grid`'s doc for the same contract on the SlotsReuse path.
    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> PaneRegionMask {
        let result = self.execute_grid_damage(model, frame, spans);
        let held = result.held;
        self.commit_pane_cache(result.cache_commit);
        held
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> PreparedGridPaint {
        self.execute_grid_shell(model, frame, GridHeaderScope::Both, || {
            let mut held = PaneRegionMask::EMPTY;
            let mut cache_commit = PreparedCacheCommit::with_capacity(4);
            for pane in PaneRegionMask::ALL.regions() {
                match self.execute_pane_damage(model, frame, pane, spans) {
                    PaneExecution::Held => held = held.with(pane),
                    PaneExecution::Untouched => {}
                    PaneExecution::Committed(commit) => cache_commit.push(commit),
                }
            }
            PreparedGridPaint { held, cache_commit }
        })
    }

    /// Paint the header corner box, gated for *correctness*: at thickness 0 it
    /// would still stroke 0.5px border lines spanning the full canvas.
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
    ///
    /// `sheet` is the committed frame's own sheet (`frame.sheet`), not
    /// another `CanvasModel::get_selected_sheet()` read — the gridline
    /// lookup runs once per grid execution and must agree with the geometry
    /// it is painting over, not with whatever the live model reports this
    /// instant.
    fn cache_show_grid(&self, model: &dyn CanvasModel, sheet: u32) {
        self.frame_cache
            .show_grid
            .set(model.get_show_grid_lines(sheet).unwrap_or(true));
    }
}

// `render_grid_blit` needs `Painter::blit` (via `BlitPainter`) to shift the
// kept band itself, so it lives in its own `BlitPainter`-bounded block,
// mirroring `GridRenderer<P: BlitPainter>`'s own split below.
impl<P: BlitPainter> RendererCore<P> {
    /// Scroll-blit variant: the ONE entry point for a blit attempt — prepares
    /// every `plan.shift_panes()` pane (see [`Self::prepare_blit`]), and only
    /// once every pane's fetch is confirmed clean does it shift a single
    /// pixel. Returns `true` (held, a complete no-op — no pixel shifted, no
    /// group opened) if any required pane's fetch reported a bridge failure;
    /// `false` once the frame actually painted.
    ///
    /// On success: shifts `plan.shifts` (the caller's `Painter::blit` used to
    /// run this same loop before calling this function; it is now internal,
    /// so a caller can never shift pixels ahead of the prepare check), then
    /// paints each prepared pane's strip/full-pane work
    /// ([`Self::execute_blit`]) inside the shared
    /// [`Self::execute_grid_shell`] sequence, narrowed to the scroll-axis
    /// header strip only (the cross-axis header is unchanged) — the same
    /// outer sequence `render_grid` uses, minus the panes the blit never
    /// visits (cross-axis panes left intact are excluded from
    /// `plan.shift_panes()`).
    pub fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        let Some(cache_commit) = self.execute_grid_blit(model, frame, plan) else {
            return true;
        };
        self.commit_pane_cache(cache_commit);
        false
    }

    pub(crate) fn execute_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> Option<PreparedCacheCommit> {
        let prepared = self.prepare_blit(model, frame, plan)?;

        // The shifts stay ahead of the shell: a held attempt must move zero
        // pixels, and a successful one must move them all before the first
        // group opens, or the repainted strips would land under stale pixels.
        for s in &plan.shifts {
            self.painter.blit(s.src, s.dst);
        }

        let cache_commit =
            self.execute_grid_shell(model, frame, GridHeaderScope::Axis(plan.axis), || {
                self.execute_blit(frame, prepared)
            });
        Some(cache_commit)
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
    pub fn render_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> PaneRegionMask {
        self.core.render_grid(model, frame, mask)
    }

    pub(crate) fn execute_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> PreparedGridPaint {
        self.core.execute_grid(model, frame, mask)
    }

    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> PaneRegionMask {
        self.core.render_grid_damage(model, frame, spans)
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> PreparedGridPaint {
        self.core.execute_grid_damage(model, frame, spans)
    }

    /// See [`RendererCore::prepare_fresh_panes`]. `pub(crate)`: an
    /// execution detail of the Fresh atomic paint path, reached only
    /// through [`crate::layer::LayerBase::paint_grid_fresh`].
    pub(crate) fn prepare_fresh_panes(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        mask: PaneRegionMask,
    ) -> Option<Vec<PreparedPane>> {
        self.core.prepare_fresh_panes(model, frame, mask)
    }

    /// See [`RendererCore::execute_fresh_grid`].
    pub(crate) fn execute_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: Vec<PreparedPane>,
    ) -> PreparedCacheCommit {
        self.core.execute_fresh_grid(model, frame, prepared)
    }

    /// Drop cached pane-buffer ranges for the masked panes. `render_pane`
    /// bulk-fetches every pane unconditionally regardless of this cache, so
    /// no orchestrator regime calls this ahead of a paint attempt today
    /// (see `Orchestrator::paint_slots_reuse_regime`'s doc for why an eager
    /// pre-paint call would be redundant with a successful commit and
    /// actively wrong on a held one). Kept as public API for a caller that
    /// wants to force a future re-grow independent of any paint attempt.
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
    /// See [`RendererCore::render_grid_blit`]. Requires `BlitPainter` (not
    /// just `Painter`) because this is now the one call that both shifts the
    /// kept band and paints the revealed strip — `LayerBase::paint_grid_blit`
    /// no longer issues `Painter::blit` itself.
    pub fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        self.core.render_grid_blit(model, frame, plan)
    }

    pub(crate) fn execute_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> Option<PreparedCacheCommit> {
        self.core.execute_grid_blit(model, frame, plan)
    }

    pub(crate) fn commit_pane_cache(&self, commit: PreparedCacheCommit) {
        self.core.commit_pane_cache(commit);
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
