//! Frame dispatch and state aggregator. Backend-agnostic; the wasm-bound
//! `IronCanvas` facade in `iron-canvas-web` owns an
//! `Orchestrator<FacadeSurface>` (`WebSurface` by default,
//! `RecordingSurface<WebSurface>` under dev-tools) and delegates every
//! setter, query, and paint call here. The model is held as
//! `Rc<dyn CanvasModel>`, so the struct carries one type parameter (the
//! `Surface`), not two.
//!
//! `paint_if_dirty` drains both layers' typed `GridSignals` and picks one
//! of five `PaintRegime` arms via `decide` (cheapness-ordered). The Fresh,
//! SlotsReuse, and Damage arms rebuild via a `Chrome::next(.., FramePath::*)`
//! walk through the matching `LayerBase` paint method; the Viewport arm goes
//! through `Chrome::next_blit`; the Overlay arm reuses
//! `last_frame` directly and repaints only the overlay. The query API
//! (`hit_test`, `cell_rect`,
//! `resize_handle_at`, `autofill_handle`) reads `last_frame`, so hits
//! agree with painted pixels by construction.

use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::CanvasModel;
use crate::chrome::{
    BlitOutcome, BlitPlan, Chrome, FramePath, FrameValidity, PaneRegion, PaneRegionMask,
};
use crate::decoration::{DecorationId, Decorations, Layer, selection::SelectionLayer};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::layer::{LayerBase, Surface};
use crate::painter::BlitPainter;
use crate::render_overlays::RenderOverlays;
use crate::renderer::{GridRenderer, OverlayRenderer};
use crate::signal::{CellDamage, GridSignals, RowSpan};
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use crate::types::ui::{HitTest, ResizeTarget};

/// Named verdict of `paint_if_dirty`'s dispatch. Each variant aligns 1:1
/// with a `paint_*` method; the regime carries everything that method
/// needs (mask, dirty bits) so `paint_if_dirty` is pure pattern-destructure.
/// Variants align with `FramePath`: `SlotsReuse` and `Fresh` here map to
/// `FramePath::SlotsReuse` and `FramePath::Fresh` inside `Chrome::next`.
#[must_use = "PaintRegime is the paint dispatch verdict; dropping it means the chosen paint_* method never runs"]
pub enum PaintRegime {
    Overlay,
    Viewport(BlitPlan),
    /// Content change whose rows are fully known: repaint only those
    /// full-width bands per pane via the blit strip machinery.
    Damage {
        spans: Vec<RowSpan>,
        signals: GridSignals,
    },
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
#[must_use = "PaintRegimeTag is the recorded regime attribution; dropping it skips a recorder frame"]
pub enum PaintRegimeTag {
    Overlay,
    Viewport,
    SlotsReuse,
    Fresh,
    Damage,
}

impl From<&PaintRegime> for PaintRegimeTag {
    fn from(r: &PaintRegime) -> Self {
        match r {
            PaintRegime::Overlay => PaintRegimeTag::Overlay,
            PaintRegime::Viewport(_) => PaintRegimeTag::Viewport,
            PaintRegime::SlotsReuse { .. } => PaintRegimeTag::SlotsReuse,
            PaintRegime::Fresh(_) => PaintRegimeTag::Fresh,
            PaintRegime::Damage { .. } => PaintRegimeTag::Damage,
        }
    }
}

/// What one pane's `render_pane*` call decided this frame. Mirrors
/// `RepaintPlan` plus the two outcomes the planner never produces, so every
/// exit from `render_pane` / `render_pane_blit` / `render_pane_strip` maps to
/// exactly one variant — the relationship `PaintRegimeTag` already has to
/// `PaintRegime`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneVerdict {
    Skip,
    Rows {
        spans: u8,
        rows: u16,
    },
    Full,
    Strip,
    /// `render_pane`'s own bridge preflight held this pane's prior buffers.
    Held,
}

impl fmt::Display for PaneVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skip => f.write_str("skip"),
            Self::Rows { spans, rows } => write!(f, "rows{spans}/{rows}"),
            Self::Full => f.write_str("FULL"),
            Self::Strip => f.write_str("strip"),
            Self::Held => f.write_str("held"),
        }
    }
}

/// Smallest band origin along one axis that shows `target` in full, given the
/// axis's frozen count, its scrollable `extent` in pixels, and where the band
/// currently starts.
///
/// The backward walk is bounded by how many slots fit in `extent`, so a jump of
/// 100k rows costs the same as a jump of one. Returning `current` unchanged is
/// the "already visible / nothing to do" answer.
fn origin_showing(
    target: i32,
    current: i32,
    frozen: i32,
    extent: i32,
    mut measure: impl FnMut(i32) -> i32,
) -> i32 {
    // A collapsed axis scrolls nowhere, and a frozen target is always painted.
    if extent <= 0 || target <= frozen {
        return current;
    }
    if target < current {
        return target; // scrolled past it — flush against the near edge
    }

    // Walk back from the target while the run still fits. `smallest` is then
    // the earliest origin that shows the target in full, so any origin at or
    // after it also shows it — hence the `max` rather than a second forward sum.
    // The loop floor also keeps `smallest` out of the frozen run, so the result
    // is a legal origin without clamping `current` on the way in.
    let mut smallest = target;
    let mut run = measure(target);
    while smallest > frozen + 1 {
        let previous = measure(smallest - 1);
        if run + previous > extent {
            break;
        }
        smallest -= 1;
        run += previous;
    }
    current.max(smallest)
}

/// Whole-frame outcome, separate from the per-pane verdicts because the blit
/// preflight aborts *before* any `render_pane*` runs: `prefetch_blit_strips`
/// returns `false` and `paint_grid_blit` returns without shifting a pixel.
/// Recording that as one pane's verdict would imply the other panes painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameOutcome {
    #[default]
    Painted,
    HeldOnBridgeFailure(PaneRegion),
}

/// A pane the blit preflight could not stage a strip for, so it fell through to
/// a whole-pane `render_pane` on a frame that was supposed to be cheap. Carries
/// the reason because the two have different fixes: a cold cache means some
/// earlier frame dropped the pane's range, while an incompatible range means
/// `shift_is_safe` rejected the geometry (for a row scroll, the visible row
/// count changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlitFallback {
    pub pane: PaneRegion,
    pub cold_cache: bool,
}

/// Per-frame attribution: which regime ran, what each pane decided, and how
/// much model traffic it cost. Written by the renderer during paint, stamped
/// into `Orchestrator.last_trace` at the end of `paint_if_dirty`.
///
/// Exists to answer "which path painted this frame?" without a code read —
/// specifically whether a post-blit `SlotsReuse` reports `Full`, which is the
/// hypothesis in `docs/designs/2026-07-24-paint-stage-remodel-and-frame-trace.md`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameTrace {
    /// `None` before the first painted frame. `PaintRegimeTag` has no
    /// `Default` on purpose — inventing one would name a regime that never ran.
    pub regime: Option<PaintRegimeTag>,
    /// The signal word `decide` acted on. Included because the regime alone
    /// cannot explain itself: `SlotsReuse` is the fallthrough arm, so seeing it
    /// tells you which arms were *rejected* only once you know which bits were
    /// raised.
    pub signals: GridSignals,
    /// Indexed by `PaneRegion as usize`. `None` = pane not visited this frame.
    pub panes: [Option<PaneVerdict>; 4],
    pub outcome: FrameOutcome,
    /// Set when a `Viewport` frame had to abandon the strip path for a pane.
    /// Still the expensive case even though `take_validated_pane_fetch` folds
    /// the two bridge crossings into one: the pane pays a whole-pane five-pass
    /// walk on a frame that was supposed to repaint a strip.
    pub blit_fallback: Option<BlitFallback>,
    /// Cell slots handed to the model: summed over the four bulk accessors and
    /// counted per call, so one 1000-cell pane fetch reads 4000. An unshiftable
    /// pane is charged once — `render_pane` adopts the buffers the preflight
    /// already validated instead of refetching the same cells.
    pub fetched_cell_slots: usize,
}

impl fmt::Display for FrameTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.regime {
            Some(r) => write!(f, "{r:?}")?,
            None => f.write_str("-")?,
        }
        write!(f, "[{:?}]", self.signals)?;
        for (i, name) in ["tl", "tr", "bl", "br"].iter().enumerate() {
            match self.panes.get(i).copied().flatten() {
                Some(v) => write!(f, " {name}:{v}")?,
                None => write!(f, " {name}:-")?,
            }
        }
        if let FrameOutcome::HeldOnBridgeFailure(pane) = self.outcome {
            write!(f, " HELD({pane:?})")?;
        }
        if let Some(fb) = self.blit_fallback {
            let why = if fb.cold_cache { "cold" } else { "range" };
            write!(f, " unshift({:?},{why})", fb.pane)?;
        }
        write!(f, " fetched={}", self.fetched_cell_slots)
    }
}

pub struct Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
    pub(crate) grid: LayerBase<S, GridRenderer<S::P>>,
    pub(crate) overlay: LayerBase<S, OverlayRenderer<S::P>>,
    theme: Rc<CanvasTheme>,
    decos: Decorations,
    model: Option<Rc<dyn CanvasModel>>,
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
    /// Row-band damage paired with `pending_content`. `CellDamage::Rows`
    /// only when every CONTENT raise since the last paint named its rows.
    pending_damage: CellDamage,
    /// Last regime `paint_if_dirty` dispatched. Stamped after `decide`,
    /// read by the recording pipeline via `last_regime()`. `None` before
    /// the first paint. Plain field — `paint_if_dirty` already holds
    /// `&mut self`, so no interior mutability is needed.
    last_regime: Option<PaintRegimeTag>,
    /// `GridSignals` drained by the last `paint_if_dirty`. Empty before
    /// the first paint.
    last_signals: GridSignals,
    /// Per-pane attribution for the last `paint_if_dirty`. Collected by the
    /// grid renderer during paint, stamped here after dispatch.
    last_trace: FrameTrace,
}

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
    pub fn new(grid_surface: S, overlay_surface: S) -> Self {
        let grid_renderer = GridRenderer::for_layer(grid_surface.clone_painter());
        let overlay_renderer = OverlayRenderer::for_layer(overlay_surface.clone_painter());
        Self {
            grid: LayerBase::new(grid_surface, grid_renderer),
            overlay: LayerBase::new(overlay_surface, overlay_renderer),
            theme: Rc::new(CanvasTheme::light()),
            decos: Decorations::default(),
            model: None,
            last_frame: None,
            size: CanvasSize { w: 0.0, h: 0.0 },
            pending_content: PaneRegionMask::EMPTY,
            pending_damage: CellDamage::Clean,
            last_regime: None,
            last_signals: GridSignals::empty(),
            last_trace: FrameTrace::default(),
        }
    }

    /// Per-pane attribution for the last `paint_if_dirty`. All-`None` panes
    /// before the first paint.
    pub fn last_trace(&self) -> FrameTrace {
        self.last_trace
    }

    /// Regime stamped by the last `paint_if_dirty`. `None` before the
    /// first paint. Read by the recording pipeline.
    pub fn last_regime(&self) -> Option<PaintRegimeTag> {
        self.last_regime
    }

    /// `GridSignals` word the last `paint_if_dirty` acted upon. Empty
    /// before the first paint.
    pub fn last_signals(&self) -> GridSignals {
        self.last_signals
    }

    /// Resize both layers in one call. No public per-layer resize, so
    /// callers can't leave the pair half-sized.
    pub fn resize(&mut self, size: CanvasSize, dpr: f64) {
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
        self.pending_damage = CellDamage::Clean;
        self.grid
            .raise(GridSignals::STRUCTURAL | GridSignals::OVERLAY);
        self.overlay.raise(GridSignals::OVERLAY);
    }

    /// Bulk-push every overlay primitive in one comparison. The per-field
    /// setters each raise OVERLAY independently; folding them into one
    /// pass lets the Leptos host's per-frame reactive memo cost a single
    /// raise instead of four.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        if self.decos.set_overlays(overlays) {
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        if self.decos.set_extend_to(target) {
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        if self.decos.set_clipboard(area) {
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        if self.decos.set_point_range(range) {
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        if self.decos.set_formula_refs(refs) {
            self.overlay.raise(GridSignals::OVERLAY);
        }
    }

    /// Install a consumer-owned overlay decoration above every built-in.
    /// The layer paints from the next frame onward — never retroactively
    /// onto a frame already emitted — and its `hit_test` runs before every
    /// built-in zone, so returning `Some` at the autofill-handle pixel
    /// steals the handle drag: stay paint-only (the trait default) unless
    /// that shadowing is intended. The registry holds a strong `Rc`; keep
    /// a typed clone, mutate through interior mutability, and call
    /// [`Self::request_overlay_repaint`] after each change — unlike the
    /// built-in setters, nothing here compares state for you.
    pub fn add_decoration(&mut self, layer: Rc<dyn Layer>) -> DecorationId {
        let id = self.decos.add_custom(layer);
        self.overlay.raise(GridSignals::OVERLAY);
        id
    }

    /// Remove a custom decoration. Removal is explicit — a layer whose
    /// consumer handle was dropped still participates in the paint and hit
    /// loops (as a no-op) until removed here. Raises `OVERLAY` only when
    /// the id was found, so a stale-id call cannot trigger a repaint.
    pub fn remove_decoration(&mut self, id: DecorationId) -> bool {
        let removed = self.decos.remove_custom(id);
        if removed {
            self.overlay.raise(GridSignals::OVERLAY);
        }
        removed
    }

    /// Push a theme. Value-compares against `self.theme` and, on change,
    /// invalidates the renderer paint cache and marks both layers dirty.
    /// `is_still_valid` now rejects a theme-mismatched frame itself, so the
    /// next paint reaches `Fresh` through the validity verdict — no out-of-band
    /// `last_frame` drop needed here. The paint-cache invalidation stays: the
    /// per-cell fingerprint cache is keyed on content, not palette, so even a
    /// Fresh rebuild would fingerprint-skip stale-color cells without it.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != *self.theme {
            self.theme = Rc::new(theme);
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

    /// Push a new data model. No `Rc::ptr_eq` dedupe: every call is
    /// treated as a change and forces the next paint to Fresh. JS-side
    /// typically pushes once per workbook, so the cost is one worst-case
    /// repaint after a redundant push.
    pub fn set_model(&mut self, model: Rc<dyn CanvasModel>) {
        self.model = Some(model);
        // `is_still_valid` doesn't see model identity, so a workbook swap
        // with the same scroll/sheet/freeze/size would otherwise reuse the
        // prev pane_set (stale row heights / column widths).
        self.last_frame = None;
        self.pending_content = PaneRegionMask::EMPTY;
        self.pending_damage = CellDamage::Clean;
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
        self.pending_damage.poison();
        self.pending_content |= mask;
        self.grid.raise(GridSignals::CONTENT);
    }

    /// Row-scoped `mark_content_dirty`: also names the damaged rows so
    /// `decide` can clip the repaint to full-width bands. Degrades to the
    /// pane-mask path whenever row info is incomplete (see `CellDamage`).
    pub fn mark_rows_damaged(&mut self, sheet: u32, span: RowSpan) {
        self.pending_damage.add_rows(sheet, span);
        self.pending_content |= PaneRegionMask::ALL;
        self.grid.raise(GridSignals::CONTENT);
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.size
    }

    pub fn theme(&self) -> &CanvasTheme {
        &self.theme
    }

    pub fn selection(&self) -> &SelectionLayer {
        self.decos.selection()
    }

    /// Surface introspection — direct access to the grid surface for
    /// callers that read or drive it outside the paint pipeline. Two
    /// consumer classes use it: this crate's recorder integration tests
    /// (inspecting emitted `DrawOp`s) and `iron-canvas-web`'s `dev-tools`
    /// recording/playback. Gated behind `surface-introspection` so the
    /// prod build doesn't carry the symbol.
    #[cfg(feature = "surface-introspection")]
    pub fn grid_surface(&self) -> &S {
        &self.grid.surface
    }

    /// Overlay-surface counterpart to [`Self::grid_surface`]; same
    /// `surface-introspection` gate and the same two consumer classes.
    #[cfg(feature = "surface-introspection")]
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
        // No live selection -> pass a zero range; the decoration layers that
        // consult `sel` (autofill, formula-refs) treat it as "no anchor"
        // and naturally fall through to the frame's pure cell hit-test.
        let sel = self.decos.selection().selection_range.unwrap_or_default();
        // Custom band first — front-to-back is reverse insertion order,
        // mirroring its paint position above every built-in.
        for (_, layer) in self.decos.custom_layers().iter().rev() {
            if let Some(hit) = layer.hit_test(frame, sel, xi, yi) {
                return hit;
            }
        }
        for layer in self.decos.hit_order() {
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
        let row = frame.pane_set.rows.pixel_to_id(yi)?;
        let col = frame.pane_set.cols.pixel_to_id(xi)?;
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

    /// Canvas-space rect of the scrollable pane — everything past the frozen
    /// bands, running to the canvas edge.
    ///
    /// Edge-triggered host behaviour (autoscroll while dragging a selection)
    /// must measure against this, not against the canvas: the near edges sit
    /// `frozen_offset` in from the origin on each axis, which is header
    /// thickness on an unfrozen sheet but header + frozen band + separator
    /// once panes are frozen. `None` before the first paint.
    /// Frozen bands wider or taller than the canvas leave no scrollable
    /// extent at all; the rect collapses to zero rather than going negative,
    /// and callers must treat a zero extent as "nothing scrolls on this axis".
    pub fn scroll_pane_rect(&self) -> Option<PixelRect> {
        let frame = self.last_frame.as_ref()?;
        let top_left = Point {
            x: frame.pane_set.cols.frozen_offset,
            y: frame.pane_set.rows.frozen_offset,
        };
        // The frame's own canvas size, not `self.size` — a resize between the
        // last paint and this query must not be mixed into a snapshot answer.
        Some(PixelRect {
            top_left,
            width: (frame.canvas_size.w as i32 - top_left.x).max(0),
            height: (frame.canvas_size.h as i32 - top_left.y).max(0),
        })
    }

    /// The scroll origin the renderer will actually honour for the model's
    /// current view — `scroll_first` applied to both axes.
    ///
    /// A scroll band never starts inside the frozen run, but nothing stops a
    /// model's `top_row` from sitting there (freezing panes does not move it).
    /// The renderer clamps silently, so the model can hold a value that
    /// disagrees with every painted pixel. Hosts write this back *before* any
    /// navigation that computes from `top_row` — page up/down derives its new
    /// selection from it, so a correction afterwards arrives too late.
    ///
    /// Reads the live model rather than the painted frame on purpose: a scroll
    /// made since the last paint is legitimate and must survive the sync.
    /// `None` when there is no model or no view.
    pub fn legal_scroll_origin(&self) -> Option<(i32, i32)> {
        let model = self.model.as_deref()?;
        let view = model.get_selected_view()?;
        let frozen_rows = model.get_frozen_rows_count(view.sheet).unwrap_or(0);
        let frozen_cols = model.get_frozen_columns_count(view.sheet).unwrap_or(0);
        Some((
            crate::geometry::slot::scroll_first(frozen_rows, view.top_row),
            crate::geometry::slot::scroll_first(frozen_cols, view.left_column),
        ))
    }

    /// Minimal `(top_row, left_column)` that brings `(row, column)` fully
    /// inside the scroll pane, or `None` when it already is (or when there is
    /// no painted frame or model to measure against).
    ///
    /// Answers from painted geometry, so it accounts for the frozen bands,
    /// measured header thickness, hidden rows and a partial trailing row —
    /// none of which the model's `window_width`/`window_height` arithmetic can
    /// see. A target inside a frozen band never scrolls its axis; a target
    /// taller or wider than the pane aligns to the pane's near edge.
    pub fn scroll_to_show(&self, row: i32, column: i32) -> Option<(i32, i32)> {
        let frame = self.last_frame.as_ref()?;
        let model = self.model.as_deref()?;
        let view = model.get_selected_view()?;
        let pane = self.scroll_pane_rect()?;

        let top = origin_showing(
            row,
            view.top_row,
            frame.pane_set.rows.frozen_count(),
            pane.height,
            |id| crate::geometry::slot::row_height(model, id),
        );
        let left = origin_showing(
            column,
            view.left_column,
            frame.pane_set.cols.frozen_count(),
            pane.width,
            |id| crate::geometry::slot::col_width(model, id),
        );
        ((top, left) != (view.top_row, view.left_column)).then_some((top, left))
    }

    /// Auto-fit width for `col`: widest formatted value across the
    /// `[first_row, last_row]` used-row span, plus padding. `None` when the
    /// model is absent or no scanned cell in `col` has text. Pure
    /// measurement — the consumer applies the returned extent.
    pub fn fit_column_width(&self, col: i32, first_row: i32, last_row: i32) -> Option<f64> {
        let model = self.model.as_deref()?;
        let metrics = self.grid.surface.painter();
        crate::autofit::fit_width(model, metrics, col, first_row, last_row)
    }

    /// Auto-fit height for `row`: tallest font across the `[first_col,
    /// last_col]` used-column span, plus padding. Same absence semantics as
    /// `fit_column_width`.
    pub fn fit_row_height(&self, row: i32, first_col: i32, last_col: i32) -> Option<f64> {
        let model = self.model.as_deref()?;
        let metrics = self.grid.surface.painter();
        crate::autofit::fit_height(model, metrics, row, first_col, last_col)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame
            .as_ref()?
            .autofill_handle(self.decos.selection().selection_range?)
    }

    /// Paint whichever layers are dirty. Dispatches via `decide` into one
    /// of five named regimes: `Overlay`, `Viewport`, `Damage`, `SlotsReuse`,
    /// `Fresh`. The `match` is exhaustive — adding a regime breaks the
    /// build here by design.
    pub fn paint_if_dirty(&mut self) {
        // Model-absent -> return *before* draining. Draining a CONTENT bit
        // raised before the first model push would lose its paired
        // `pending_content` / `pending_damage` state, breaking the
        // `pending_content ⟺ CONTENT` invariant the next real paint relies on.
        if self.model.is_none() {
            return;
        }
        let signals = self.grid.drain_signals() | self.overlay.drain_signals();
        if signals.is_empty() {
            // Nothing drained, model never taken — nothing to restore.
            return;
        }
        // Lift the model out so the paint methods can take `&mut self`
        // without overlapping the model borrow. The `is_none` guard above
        // makes the `else` unreachable, but `let-else` keeps it panic-free.
        let Some(model) = self.model.take() else {
            return;
        };

        let model_dyn: &dyn CanvasModel = model.as_ref();
        let regime = self.decide(signals, model_dyn);
        self.last_regime = Some(PaintRegimeTag::from(&regime));
        self.last_signals = signals;
        // Clear before dispatch so the trace describes this frame only. An
        // `Overlay` regime legitimately leaves every pane `None` — it never
        // calls a grid pane renderer.
        self.grid.renderer.reset_trace();
        match regime {
            PaintRegime::Overlay => self.paint_overlay_regime(model_dyn),
            PaintRegime::Viewport(plan) => self.paint_viewport_regime(model_dyn, plan),
            PaintRegime::SlotsReuse { mask, signals } => {
                self.paint_slots_reuse_regime(model_dyn, mask, signals)
            }
            PaintRegime::Fresh(signals) => self.paint_fresh_regime(model_dyn, signals),
            PaintRegime::Damage { spans, signals } => {
                self.paint_damage_regime(model_dyn, spans, signals)
            }
        }
        self.pending_content = PaneRegionMask::EMPTY;
        self.pending_damage = CellDamage::Clean;

        self.last_trace = self.grid.renderer.trace();
        self.last_trace.regime = self.last_regime;
        self.last_trace.signals = signals;

        // Single restore site.
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
                f.is_still_valid(model, self.size, &self.theme)
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
        if !content_dirty
            && let Some(active) = self.decos.selection().active_cell.as_ref()
            && let Some(frame) = self.last_frame.as_ref()
            && let Some(plan) = frame.screen_for_blit(model, self.size, &self.theme, active)
        {
            return PaintRegime::Viewport(plan);
        }

        // Damage fast path: viewport reusable, every CONTENT raise named its
        // rows, and they were recorded against the sheet still on screen.
        // STRUCTURAL bars the arm — band-clipping must not paper over a
        // geometry/theme change that happens to keep SlotsReuse validity.
        if content_dirty
            && !sig.contains(GridSignals::STRUCTURAL)
            && matches!(validity, FrameValidity::SlotsReuse)
            && let Some(frame) = self.last_frame.as_ref()
            && let CellDamage::Rows { sheet, spans } = &self.pending_damage
            && *sheet == frame.sheet
        {
            return PaintRegime::Damage {
                spans: spans.clone(),
                signals: sig,
            };
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

    /// Overlay-only fast path. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. `decide` proves the
    /// preconditions (slot vecs still match, `last_frame` is `Some`).
    fn paint_overlay_regime(&mut self, model: &dyn CanvasModel) {
        self.decos.refresh_overlay_state(model);
        let Some(prev) = self.last_frame.as_ref() else {
            return;
        };
        self.overlay.paint_overlay_layer(
            model,
            prev,
            self.decos.selection(),
            &self.decos.overlay_slice(),
            self.decos.custom_layers(),
        );
        self.overlay.present();
    }

    /// Scroll-blit fast path. `decide` already filtered no-op scrolls and
    /// viewport shifts where the kept band can't be reused; we trust the
    /// verdict and the supplied plan. Always repaints the overlay too —
    /// a viewport shift moves every overlay primitive's pixel position.
    ///
    /// `Chrome::next_blit` may demote to `Fresh` when in-place reuse rejects
    /// (e.g. row-header digit boundary). The `BlitOutcome` variant we get back
    /// *is* the dispatch — the `FreshFallback` arm takes the full repaint with
    /// cache invalidation, instead of a `paint_grid_blit` that would carry
    /// stale per-pane caches against the freshly rebuilt slot vecs.
    fn paint_viewport_regime(&mut self, model: &dyn CanvasModel, plan: BlitPlan) {
        let Some(prev) = self.last_frame.take() else {
            return;
        };
        let frame = match Chrome::next_blit(Some(prev), model, self.size, &self.theme, &plan) {
            BlitOutcome::Blitted(frame) => {
                self.grid.paint_grid_blit(model, &frame, &plan);
                frame
            }
            BlitOutcome::FreshFallback(frame) => {
                self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
                self.grid.invalidate_paint_cache();
                self.grid.paint_grid(model, &frame);
                frame
            }
        };
        self.grid.present();
        self.decos.refresh_overlay_state(model);
        self.overlay.paint_overlay_layer(
            model,
            &frame,
            self.decos.selection(),
            &self.decos.overlay_slice(),
            self.decos.custom_layers(),
        );
        self.overlay.present();
        self.last_frame = Some(frame);
    }

    /// Damage regime: slot vecs survive (same preconditions as SlotsReuse),
    /// prior grid pixels stay, only the damaged bands refetch + repaint.
    /// No cache invalidation here — the strip path (`render_pane_strip`)
    /// splices fetched bands into the pane buffers and invalidates the pane
    /// fingerprint itself, atomically: a transient bridge failure on any of
    /// the four strip buffers leaves that pane's buffers, pixels, range,
    /// and tree untouched instead of partially splicing.
    fn paint_damage_regime(
        &mut self,
        model: &dyn CanvasModel,
        spans: Vec<RowSpan>,
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
            FramePath::SlotsReuse {
                stale_panes: PaneRegionMask::EMPTY,
            },
        );
        self.grid.paint_grid_damage(model, &frame, &spans);
        self.grid.present();
        self.decos.refresh_overlay_state(model);
        // CONTENT is implied in this arm, so the active-cell-repaint hook
        // fires unconditionally — same reasoning as the SlotsReuse arm.
        if signals.overlay_dirty() || self.decos.active_cell_repaint().is_some() {
            self.overlay.paint_overlay_layer(
                model,
                &frame,
                self.decos.selection(),
                &self.decos.overlay_slice(),
                self.decos.custom_layers(),
            );
            self.overlay.present();
        }
        self.last_frame = Some(frame);
    }

    /// SlotsReuse regime: prev's slot vecs survive (viewport unchanged);
    /// only `pane_cache` entries inside `mask` are invalidated so
    /// `render_pane` refetches there. Unmasked panes fingerprint-skip.
    /// `invalidate_pane_cache` drops buffer *ranges* only, never painted
    /// trees — a masked pane whose refetch matches its prior content still
    /// fingerprint-skips (see `PaneCache::invalidate`'s doc).
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
            FramePath::SlotsReuse { stale_panes: mask },
        );

        self.grid.invalidate_pane_cache(mask);
        self.grid.invalidate_paint_cache();

        self.grid.paint_grid(model, &frame);
        self.grid.present();
        // Refresh the selection snapshot unconditionally: even on a
        // CONTENT-only signal the grid just repainted with new values,
        // so the next paint's `screen_for_blit` must compare against
        // the post-edit hash.
        self.decos.refresh_overlay_state(model);
        // Active-cell-repaint hook paints model-derived pixels on the
        // overlay — so CONTENT implies OVERLAY when an active cell exists.
        // Without this, DEL on the active cell clears the model but the
        // overlay still shows the old value on top of the grid.
        let must_paint_overlay = signals.overlay_dirty()
            || (signals.contains(GridSignals::CONTENT)
                && self.decos.active_cell_repaint().is_some());
        if must_paint_overlay {
            self.overlay.paint_overlay_layer(
                model,
                &frame,
                self.decos.selection(),
                &self.decos.overlay_slice(),
                self.decos.custom_layers(),
            );
            self.overlay.present();
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
        self.grid.present();
        self.decos.refresh_overlay_state(model);
        if signals.overlay_dirty() {
            self.overlay.paint_overlay_layer(
                model,
                &frame,
                self.decos.selection(),
                &self.decos.overlay_slice(),
                self.decos.custom_layers(),
            );
            self.overlay.present();
        }
        self.last_frame = Some(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::origin_showing;

    /// Uniform rows, so `extent / 20` is how many fit and every expectation
    /// below is arithmetic a reader can redo in their head.
    fn rows_20(_id: i32) -> i32 {
        20
    }

    #[test]
    fn stays_put_when_there_is_nothing_to_scroll() {
        // A collapsed axis has nowhere to put the target.
        assert_eq!(origin_showing(50, 7, 0, 0, rows_20), 7);
        assert_eq!(origin_showing(50, 7, 0, -100, rows_20), 7);
        // A frozen target is painted whatever the scrollable band shows.
        assert_eq!(origin_showing(2, 7, 3, 500, rows_20), 7);
    }

    #[test]
    fn flushes_against_the_near_edge_when_scrolled_past() {
        assert_eq!(origin_showing(5, 20, 0, 500, rows_20), 5);
    }

    /// The trailing `max` earns its keep here: the walk finds row 8 as the
    /// earliest origin that fits, but the band already sits at 10 and already
    /// shows row 12 — scrolling back to 8 would be visible, pointless motion.
    #[test]
    fn leaves_an_already_visible_target_alone() {
        assert_eq!(origin_showing(12, 10, 0, 100, rows_20), 10);
    }

    /// A target past the far edge pulls the origin forward — to the *smallest*
    /// origin that still shows the target whole, not merely to the target.
    #[test]
    fn walks_back_to_the_smallest_origin_that_shows_the_target() {
        // Five 20 px rows fill 100, so 26..=30 is the earliest band showing 30.
        // An implementation that stopped after one step would answer 29.
        assert_eq!(origin_showing(30, 2, 0, 100, rows_20), 26);
    }

    /// Rows of 8/19/30/41/52 px on a five-row cycle, so a walk that assumed a
    /// uniform height cannot land on the right origin by symmetry.
    fn rows_uneven(id: i32) -> i32 {
        8 + id.rem_euclid(5) * 11
    }

    /// The walk accumulates real heights: rows 30 (8 px) and 29 (52 px) fill 60
    /// of the 100 px band, and taking row 28 (41 px) too would overflow it.
    #[test]
    fn walk_sums_actual_row_heights_rather_than_assuming_uniform_rows() {
        assert_eq!(origin_showing(30, 2, 0, 100, rows_uneven), 29);
    }
}
