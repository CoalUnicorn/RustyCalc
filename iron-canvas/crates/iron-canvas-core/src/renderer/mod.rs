//! Renderer core for the spreadsheet grid.
//!
//! # Lifecycle
//!
//! `Orchestrator<S>` (in [`crate::orchestrator`]) owns two
//! [`LayerBase<S, R>`](crate::layer::LayerBase) values: one for the grid,
//! one for the overlay. Each `LayerBase` holds a [`Surface`](crate::layer::Surface)
//! and a layer-specific renderer wrapping [`RendererCore`] — and no dirty
//! state of its own. In the wasm build the surface is
//! `iron_canvas_canvas2d::WebSurface`; both contexts keep alpha
//! (`alpha: true`) so empty buffers stay transparent. Sheet backgrounds
//! must remain opaque because background fills erase retained content.
//! The overlay also uses `desynchronized: true`. The renderer is long-lived per
//! layer, so the painter's cached fill/stroke/font/line-width state
//! survives across frames.
//!
//! State pushes from the host mark work on `Orchestrator`'s single pending
//! value. `Orchestrator::render_pending` picks a strategy from it and drives
//! the layers that strategy paints through their `LayerBase` paint method:
//! `paint_grid` / `paint_grid_blit` for the grid, `paint_overlay_layer`
//! for the overlay. The grid path calls into [`RendererCore::render_grid`];
//! the overlay path iterates the [`Layer`](crate::decoration::Layer)
//! decorations in `crate::decoration` and calls back into `RendererCore`
//! for the active-cell repaint and header highlights.
//!
//! # Render pipeline
//!
//! Two paint entry points, each driven by `render_pending` per strategy:
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
//! whole segment before explicit borders, so an explicit `right` on cell A
//! wins over cell B's grid `left` at the shared pixel column. Text runs last;
//! text that would escape its cell is clipped by the text painter.
//!
//! # Frozen panes
//!
//! The grid splits into up to four quadrants (`TopLeft`, `TopRight`,
//! `BottomLeft`, `BottomRight`) based on frozen rows and columns. Each
//! quadrant is rendered as one segment of the grid walk against a different
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

pub mod blit;
pub mod cache;
pub mod cell;
pub mod chrome;
pub mod diagnostics;
pub mod prepared;
mod trace;
// `renderer/overlay/` has moved to `src/decoration/`. Each decoration is
// a struct that impls `Layer`; the orchestration that used to live in
// `RendererCore::render_overlays` is now in
// `LayerBase::paint_overlay_layer` (src/layer/mod.rs).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::CanvasModel;
use crate::chrome::Chrome;
#[cfg(feature = "dev-diagnostics")]
use crate::chrome::GridLayout;
pub use crate::chrome::PaneRegion;
use crate::frame::BlitPlan;
use crate::frame::work::RowSpan;
use crate::geometry::prim::Axis;
use crate::renderer::cache::{FrameCache, GridCache};
pub use cache::ColorIntern;
pub use cache::FontIntern;

pub use self::cell::text::{TextLine, layout_into};

use crate::frame::FrameTrace;
use crate::painter::{BlitPainter, GroupClass, Painter};
pub(crate) use crate::renderer::prepared::GridCacheCommit;
use crate::renderer::prepared::{FetchedCells, PreparedGrid};

// The successful arm deliberately owns the fixed-size cache commit; boxing
// it would add one allocation to every retained-paint frame.
#[must_use]
#[allow(clippy::large_enum_variant)]
pub(crate) enum GridPaintOutcome {
    /// The grid executed; owns the aggregate cache commit for the
    /// completion boundary to install.
    Committed(GridCacheCommit),
    /// A bridge failure held the whole grid before any paint or cache
    /// mutation; no commit exists.
    Held,
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
    /// Renderer-lifetime exact-layout grid buffers and fingerprint truth.
    pub grid_cache: GridCache,
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
    /// Dev-only structured capture state. `pub(crate)` so the gated
    /// capture methods in `renderer::diagnostics` can read it; zero-size
    /// contribution to production builds (feature-gated), and all writes
    /// are no-ops while its `enabled` flag is false.
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) diag: diagnostics::DiagState,
}

impl<P: Painter> RendererCore<P> {
    pub fn painter(&self) -> &P {
        self.painter.as_ref()
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
            grid_cache: GridCache::default(),
            font_intern: FontIntern::new(),
            color_intern: ColorIntern::new(),
            trace: Cell::new(FrameTrace::default()),
            #[cfg(feature = "dev-diagnostics")]
            diag: diagnostics::DiagState::default(),
        }
    }

    /// Paint all visible grid segments for a slot-reusing frame.
    ///
    /// Returns `true` when any segment reports a bridge failure. A held call
    /// performs no painter work and installs no cache state; `false` means the
    /// attempt completed (including a fingerprint skip) and any owned cache
    /// commit was installed.
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        match self.execute_grid(model, frame) {
            GridPaintOutcome::Committed(commit) => {
                self.commit_grid_cache(commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    /// The grid group sequence every strategy shares:
    /// `Grid -> Cells -> FrozenSep -> Headers -> Corner`. Only the work
    /// inside Cells differs between ChangedCells, DamagedRows, FullRebuild,
    /// and ScrollBlit,
    /// so `execute_cells` owns that and nothing else; its typed result
    /// passes straight back out.
    ///
    /// `execute_cells` runs only after whole-grid preflight, so the groups
    /// opened here always close.
    fn execute_grid_shell<T>(
        &self,
        frame: &Chrome,
        headers: GridHeaderScope,
        execute_cells: impl FnOnce() -> T,
    ) -> T {
        self.painter.begin_group(GroupClass::Grid);

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

    pub(crate) fn execute_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_full_grid(model, frame) else {
            return GridPaintOutcome::Held;
        };
        let commit = self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(commit)
    }

    /// Preflight every grid segment for a Fresh frame without touching the
    /// painter or committed cache state. A bridge failure returns `None` only
    /// after all earlier prepared bundles have been recycled.
    pub(crate) fn prepare_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> Option<PreparedGrid> {
        self.prepare_full_grid(model, frame)
    }

    /// Execute a fully preflighted Fresh grid. The returned owned commit is
    /// installed only by the caller's successful completion boundary. The
    /// grid-line config was fetched by the caller before any painter op.
    pub(crate) fn execute_fresh_grid(
        &self,
        _model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: PreparedGrid,
    ) -> GridCacheCommit {
        self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        })
    }

    /// Combined Fresh prepare and execute. Returns `true` with zero painter
    /// interaction on any bridge failure; otherwise commits the whole grid.
    pub fn render_grid_fresh(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return true;
        }
        let Some(prepared) = self.prepare_fresh_grid(model, frame) else {
            return true;
        };
        let cache_commit = self.execute_fresh_grid(model, frame, prepared);
        self.commit_grid_cache(cache_commit);
        false
    }

    /// Damage variant: prior pixels stay; only the damaged full-width row
    /// bands refetch and repaint across every intersecting grid segment. The
    /// outer sequence is not restated
    /// here — it is the shared [`Self::execute_grid_shell`] `render_grid`
    /// also runs through, which is what guarantees the frozen separators
    /// still paint after the cells (winning their pixels back from the
    /// band's re-stroked grid lines at the freeze boundary).
    ///
    /// Returns `true` when any strip reports a bridge failure. The whole grid
    /// is held atomically and no cache state is installed; see
    /// [`Self::render_grid`] for the same SlotsReuse contract.
    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> bool {
        match self.execute_grid_damage(model, frame, spans) {
            GridPaintOutcome::Committed(commit) => {
                self.commit_grid_cache(commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_damage_grid(model, frame, spans) else {
            return GridPaintOutcome::Held;
        };
        let commit = self.execute_grid_shell(frame, GridHeaderScope::Both, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(commit)
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

    /// Resolve the per-sheet grid-line toggle once per grid execution and
    /// cache it for the hot per-cell `paint_borders_grid` walk. `Absent`
    /// (no override) selects the documented default — show, matching
    /// Excel's default-on. `BridgeFailed` returns `None`: the caller must
    /// hold the attempt before any painter op — painting grid lines (or not)
    /// from a fabricated answer is a config lie, not a pixel choice.
    ///
    /// `sheet` is the committed frame's own sheet (`frame.sheet`), not
    /// another `CanvasModel::get_selected_sheet()` read — the gridline
    /// lookup runs once per grid execution and must agree with the geometry
    /// it is painting over, not with whatever the live model reports this
    /// instant.
    pub(crate) fn fetch_show_grid(&self, model: &dyn CanvasModel, sheet: u32) -> Option<bool> {
        let show = match model.get_show_grid_lines(sheet) {
            crate::types::fetched::Fetched::Value(v) => v,
            crate::types::fetched::Fetched::Absent => true,
            crate::types::fetched::Fetched::BridgeFailed => return None,
        };
        self.frame_cache.show_grid.set(show);
        Some(show)
    }
}

// `render_grid_blit` needs `Painter::blit` (via `BlitPainter`) to shift the
// kept band itself, so it lives in its own `BlitPainter`-bounded block,
// mirroring `GridRenderer<P: BlitPainter>`'s own split below.
impl<P: BlitPainter> RendererCore<P> {
    /// Preflight every candidate-derived address strip before applying the
    /// plan's single pixel shift. A bridge failure returns `true` without a
    /// blit, group bracket, paint, or cache mutation. Compatible shifts repaint
    /// only the scroll-axis header; full-grid fallback repaints both headers.
    pub fn render_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> bool {
        match self.execute_grid_blit(model, frame, plan) {
            GridPaintOutcome::Committed(cache_commit) => {
                self.commit_grid_cache(cache_commit);
                false
            }
            GridPaintOutcome::Held => true,
        }
    }

    pub(crate) fn execute_grid_blit(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        plan: &BlitPlan,
    ) -> GridPaintOutcome {
        if self.fetch_show_grid(model, frame.sheet).is_none() {
            return GridPaintOutcome::Held;
        }
        let Some(prepared) = self.prepare_blit_grid(model, frame, plan) else {
            return GridPaintOutcome::Held;
        };
        let is_shift = matches!(&prepared, PreparedGrid::Blit { .. });

        // The shifts stay ahead of the shell: a held attempt must move zero
        // pixels, and a successful one must move them all before the first
        // group opens, or the repainted strips would land under stale pixels.
        if is_shift {
            self.painter.blit(plan.shift.src, plan.shift.dst);
        }

        let headers = if is_shift {
            GridHeaderScope::Axis(plan.axis)
        } else {
            GridHeaderScope::Both
        };
        let cache_commit = self.execute_grid_shell(frame, headers, || {
            self.execute_prepared_grid(frame, prepared)
        });
        GridPaintOutcome::Committed(cache_commit)
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
    pub fn render_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> bool {
        self.core.render_grid(model, frame)
    }

    pub(crate) fn execute_grid(&self, model: &dyn CanvasModel, frame: &Chrome) -> GridPaintOutcome {
        self.core.execute_grid(model, frame)
    }

    pub fn render_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> bool {
        self.core.render_grid_damage(model, frame, spans)
    }

    pub(crate) fn execute_grid_damage(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        spans: &[RowSpan],
    ) -> GridPaintOutcome {
        self.core.execute_grid_damage(model, frame, spans)
    }

    /// See [`RendererCore::fetch_show_grid`]. `pub(crate)` so the layer's
    /// grid paint entries can hold before a bg fill or pixel shift.
    pub(crate) fn fetch_show_grid(&self, model: &dyn CanvasModel, sheet: u32) -> Option<bool> {
        self.core.fetch_show_grid(model, sheet)
    }

    /// See [`RendererCore::prepare_fresh_grid`]. `pub(crate)`: an
    /// execution detail of the Fresh atomic paint path, reached only
    /// through [`crate::layer::LayerBase::paint_grid_fresh`].
    pub(crate) fn prepare_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
    ) -> Option<PreparedGrid> {
        self.core.prepare_fresh_grid(model, frame)
    }

    /// See [`RendererCore::execute_fresh_grid`].
    pub(crate) fn execute_fresh_grid(
        &self,
        model: &dyn CanvasModel,
        frame: &Chrome,
        prepared: PreparedGrid,
    ) -> GridCacheCommit {
        self.core.execute_fresh_grid(model, frame, prepared)
    }

    /// Mark retained pixels and cell buffers stale while keeping their
    /// allocations available for the next successful grid preparation.
    pub fn invalidate_grid_buffers(&self) {
        self.core.grid_cache.invalidate_buffers();
    }

    pub fn reset_trace(&self) {
        self.core.reset_trace();
    }

    pub fn trace(&self) -> FrameTrace {
        self.core.trace()
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn set_diag_enabled(&self, enabled: bool) {
        self.core.set_diag_enabled(enabled);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn last_diag(&self) -> Option<diagnostics::FrameDiagnostics> {
        self.core.last_diag()
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_reset_capture(&self) {
        self.core.diag_reset_capture();
    }
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_begin_attempt(
        &self,
        delta: diagnostics::DiagDeltaKind,
        rebuild_reason: Option<crate::frame::RebuildReason>,
    ) {
        self.core.diag_begin_attempt(delta, rebuild_reason);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn diag_blit(
        &self,
        plan: &BlitPlan,
        result: diagnostics::DiagBlitResultTag,
        cold_cache: Option<bool>,
        previous: Option<GridLayout>,
        candidate: GridLayout,
    ) {
        self.core
            .diag_blit(plan, result, cold_cache, previous, candidate);
    }

    #[cfg(feature = "dev-diagnostics")]
    pub(crate) fn publish_diag(&self, completion: diagnostics::DiagCompletion) {
        self.core.publish_diag(completion);
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
    ) -> GridPaintOutcome {
        self.core.execute_grid_blit(model, frame, plan)
    }

    pub(crate) fn commit_grid_cache(&self, commit: GridCacheCommit) {
        self.core.commit_grid_cache(commit);
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
        cell: crate::types::coord::CellCoord,
        frame: &Chrome,
    ) {
        self.core.repaint_active_cell(model, cell, frame);
    }
}

impl<P: Painter> LayerOps for OverlayRenderer<P> {
    type Painter = P;
    fn resize_for_dpr(&mut self, dpr: f64) {
        self.core.resize_for_dpr(dpr);
    }
}
