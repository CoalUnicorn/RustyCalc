//! Frame dispatch and state aggregator. Backend-agnostic; the wasm-bound
//! `IronCanvas` facade in `iron-canvas-web` owns an
//! `Orchestrator<FacadeSurface>` (`WebSurface` by default,
//! `RecordingSurface<WebSurface>` under dev-tools) and delegates every
//! setter, query, and paint call here. The model is held as
//! `Rc<dyn CanvasModel>`, so the struct carries one type parameter (the
//! `Surface`), not two.
//!
//! `render_pending` takes the single queued `PendingWork` value, classifies
//! the attempt's geometric delta via `Chrome::classify`, and turns both into
//! one closed `FramePlan` via the pure `plan_frame` function — the complete
//! `PendingWork` x `FrameDelta` table lives on that function's doc comment.
//! The plan's `GridWork` selects one of five render methods.
//! The strategy order is `OverlayOnly`, `ScrollBlit`, `DamagedRows`,
//! `ChangedCells`, and `FullRebuild`.
//!
//! Each render method prepares (bulk bridge reads, no mutation of
//! committed state) and executes (paints into the backing target) its own
//! grid transaction, returning its aggregate `GridCacheCommit` as data, then
//! reduces to one private `AttemptOutcome` — overlay or grid committed, or
//! held — instead of advancing `last_frame`, presenting a surface, or
//! touching `self.pending` itself.
//! [`Orchestrator::finish_attempt`] is the single completion boundary every
//! outcome flows through: it installs the attempt-owned cache commit,
//! preserves or replaces `last_frame`, presents whichever layers actually
//! painted, merges retry work back into
//! `self.pending`, and publishes
//! `last_strategy`/`last_effective_strategy`/`last_work_flags`/`last_trace`.
//! A bridge failure during a strategy's bulk fetch therefore gives a clean
//! whole-grid `Held` outcome rather than a partially-applied side effect.
//!
//! The query API (`hit_test`, `cell_rect`, `resize_handle_at`,
//! `autofill_handle`) reads `last_frame`, so hits agree with painted pixels
//! by construction — including while an attempt is held, since `last_frame`
//! never advances to a candidate whose bridge reads never confirmed clean.
//!
//! Work ownership is entirely here: every setter marks intent on
//! `self.pending`, and a paint attempt consumes it with one
//! `mem::take`. Layers hold no dirty state.

use std::rc::Rc;

use crate::CanvasModel;
use crate::autofit::AutoFitError;
use crate::chrome::{Chrome, FramePath, FreshBuild, PreparedBlitOutcome, RecycledSlots};
use crate::decoration::{DecorationId, Decorations, Layer, selection::SelectionLayer};
use crate::frame::work::{PendingWork, RowSpan, WorkFlags};
use crate::frame::{
    BlitPlan, FrameInputFailure, FrameInputs, FrameOutcome, FrameTrace, GridWork, OverlayWork,
    PaintResult, RenderStrategy, plan_frame,
};
use crate::geometry::CanvasMetrics;
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::layer::{LayerBase, Surface};
use crate::painter::BlitPainter;
use crate::render_overlays::RenderOverlays;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::DiagCompletion;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::{
    DiagBlitResultTag, DiagCacheResolution, DiagDeltaKind, DiagPaintedLayers, FrameDiagnostics,
};
use crate::renderer::{GridCacheCommit, GridPaintOutcome, GridRenderer, OverlayRenderer};
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use crate::types::ui::{HitTest, ResizeTarget};

/// Smallest band origin along one axis that shows `target` in full, given the
/// axis's frozen count, its scrollable `extent` in pixels, and where the band
/// currently starts.
///
/// The backward walk is bounded by how many slots fit in `extent`, so a jump of
/// 100k rows costs the same as a jump of one. Returning `current` unchanged is
/// the "already visible / nothing to do" answer. `None` when `measure`
/// reports a transient bridge failure — a scroll target derived from
/// fabricated heights is not trustworthy.
fn origin_showing(
    target: i32,
    current: i32,
    frozen: i32,
    extent: i32,
    mut measure: impl FnMut(i32) -> Option<i32>,
) -> Option<i32> {
    // A collapsed axis scrolls nowhere, and a frozen target is always painted.
    if extent <= 0 || target <= frozen {
        return Some(current);
    }
    if target < current {
        return Some(target); // scrolled past it — flush against the near edge
    }

    // Walk back from the target while the run still fits. `smallest` is then
    // the earliest origin that shows the target in full, so any origin at or
    // after it also shows it — hence the `max` rather than a second forward sum.
    // The loop floor also keeps `smallest` out of the frozen run, so the result
    // is a legal origin without clamping `current` on the way in.
    let mut smallest = target;
    let mut run = measure(target)?;
    while smallest > frozen + 1 {
        let previous = measure(smallest - 1)?;
        if i64::from(run) + i64::from(previous) > i64::from(extent) {
            break;
        }
        smallest -= 1;
        run += previous;
    }
    Some(current.max(smallest))
}

/// What `finish_attempt` does to `Orchestrator::last_frame` for one
/// outcome. `Preserve` covers two distinct cases that both mean "do not
/// touch the field": an `Overlay` attempt never had a candidate to begin
/// with, and an atomically-held `Fresh` attempt deliberately never took
/// `last_frame` out of `self` during preparation (see `render_full_rebuild`)
/// so there is nothing to put back.
// `Chrome` is large and intentionally carried by value here, matching
// `chrome::blit`'s own `#[allow(clippy::large_enum_variant)]` precedent on
// `PreparedBlitOutcome`: boxing a variant would add a heap allocation to
// every committed paint attempt, not just the rare held/rollback path
// clippy's size comparison is really about.
#[allow(clippy::large_enum_variant)]
enum FrameUpdate {
    Preserve,
    Replace(Chrome),
}

/// This attempt's already-captured `FrameInputs` sample plus the model and
/// `plan_frame`'s `OverlayWork` verdict — everything `finish_attempt` needs
/// to refresh and (conditionally) repaint the overlay against the frame
/// that will be committed. `None` only for the capture-failure attempt,
/// which never reaches a strategy and so never refreshes overlay state at
/// all. Bundled rather than three loose parameters so it is impossible to
/// pass `inputs` without the `model`/`overlay_work` it was captured
/// alongside.
struct OverlayContext<'a> {
    model: &'a dyn CanvasModel,
    inputs: &'a FrameInputs,
    work: OverlayWork,
}

/// Private completion outcome for one paint attempt — the one value every
/// strategy preparation/execution helper reduces to, and the only
/// thing `finish_attempt` accepts. The variants close the outcome algebra:
/// a committed attempt either never touched the grid (`OverlayCommitted`)
/// or owns the grid cache commit it installed (`GridCommitted`), so a grid
/// strategy can never commit without its commit and the overlay arm can
/// never carry one. `Held` carries only held causes (`HoldReason`), so a
/// held attempt can never report a committed outcome.
///
/// `frame` on a `Held` outcome is not always `Preserve`: a strategy that *did*
/// take ownership of `last_frame` to build its candidate (`ScrollBlit`'s
/// blit, or a `ScrollBlit`-selected `FreshFallback`) must hand back an
/// equivalent value — the alternative would leave `last_frame` stuck at
/// `None` for the rest of the attempt's synchronous call chain. Every
/// `FrameUpdate` a strategy constructs here is either `Preserve` (nothing was
/// ever taken) or an already-resolved, zero-clone value the strategy had to
/// build anyway to decide Held in the first place; `finish_attempt` remains
/// the one function that performs the actual `self.last_frame = ..`
/// assignment.
//
// Keeping the prepared cache transaction inline avoids a per-frame box
// allocation. The public `FrameOutcome`, painted layers, commit sequence,
// and cache resolution are all derived from this one value in
// `finish_attempt`.
#[must_use]
#[allow(clippy::large_enum_variant)]
enum AttemptOutcome {
    /// Overlay-only attempt: no grid candidate, no cache commit; the
    /// committed `Chrome` is preserved as-is.
    OverlayCommitted,
    /// A grid strategy executed; owns the aggregate cache commit for
    /// `finish_attempt` to install.
    GridCommitted {
        cache_commit: GridCacheCommit,
        frame: FrameUpdate,
        effective: RenderStrategy,
    },
    /// Nothing executed and nothing may be presented, cached, or observed
    /// as a geometry change.
    Held {
        retry: PendingWork,
        frame: FrameUpdate,
        reason: HoldReason,
    },
}

/// Why a paint attempt was held. Only held causes exist here — a `Held`
/// outcome can never carry a committed reason.
// The shared `Failure` postfix is the point: every variant names a held cause.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HoldReason {
    /// `FrameInputs::capture` failed before dispatch reached a strategy at
    /// all — no candidate geometry, no cache invalidation, no paint.
    InputFailure(FrameInputFailure),
    /// A bulk bridge fetch failed during a strategy's prepare phase; the
    /// whole attempt is requeued.
    BridgeFailure,
}

impl HoldReason {
    /// Exact public outcome projection for `FrameTrace`.
    fn frame_outcome(&self) -> FrameOutcome {
        match self {
            HoldReason::InputFailure(failure) => FrameOutcome::HeldOnInputFailure(*failure),
            HoldReason::BridgeFailure => FrameOutcome::HeldOnBridgeFailure,
        }
    }
}

/// A bridge hold invalidates the attempted content transaction as a whole.
/// Preserve any coalesced geometry/view/overlay intent, but widen content to
/// the full visible grid before merging the retry back into pending work.
fn retry_grid_wide(mut work: PendingWork) -> PendingWork {
    work.mark_all_content();
    work
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
    /// Advanced (wrapping) by every `set_model`. Captured into `FrameInputs`
    /// so a later classifier can report `Rebuild(Model)` for an ordinary
    /// model replacement without comparing trait-object pointers.
    /// Correctness never depends on uniqueness after a wrap: `set_model`
    /// independently queues geometry work every time, so this exists to
    /// classify and diagnose, not to gate repaint.
    model_generation: u64,
    last_frame: Option<Chrome>,
    /// Standing pool of slot-Vec allocations for `FramePath::Fresh`
    /// construction, owned here (not derived from `last_frame` inline) so a
    /// Fresh candidate can be built via `Chrome::build` without touching
    /// `last_frame`'s own pane_set at all until the candidate is confirmed
    /// good — see `chrome::recycled_slots`'s module doc. `render_full_rebuild`
    /// takes this pool's vectors to build, then folds the *outgoing*
    /// committed frame's vectors back in once the candidate has replaced it.
    spare_slots: RecycledSlots,
    /// Validated canvas metrics from the last `resize`. `None` before the
    /// first resize — not a zero sentinel, since `resize` must self-invalidate
    /// on the very first call regardless of what values it is given. Private
    /// and unexposed: the wasm facade keeps its own DPR copy for the
    /// recording/playback pipeline.
    metrics: Option<CanvasMetrics>,
    /// Everything queued for the next paint attempt: geometry rebuild, view
    /// movement, content damage, overlay repaint. The single owner of paint
    /// work — layers hold none. Every setter marks intent here; a paint
    /// attempt consumes it with one `mem::take`, so successful consumption
    /// needs no end-of-paint clearing assignment. Only a strategy's own retry
    /// rule merges work back in.
    pending: PendingWork,
    /// Last strategy that `render_pending` dispatched. Stamped from
    /// `plan.grid.strategy()` after `plan_frame`, read by the
    /// recording pipeline via `last_strategy()`. `None` before
    /// the first paint. Plain field — `render_pending` already holds
    /// `&mut self`, so no interior mutability is needed.
    last_strategy: Option<RenderStrategy>,
    /// The strategy that actually ran after dispatch can override its
    /// own selection (see `FrameTrace::effective`). Set to `last_strategy`'s
    /// value at dispatch; `render_scroll_blit`'s `FreshFallback` arm is
    /// the only site that overwrites it afterward.
    last_effective_strategy: Option<RenderStrategy>,
    /// Diagnostic projection of the work the last `render_pending` took.
    /// Empty before the first paint.
    last_work_flags: WorkFlags,
    /// Grid-wide attribution for the last `render_pending`. Collected by the
    /// grid renderer during paint, stamped here after dispatch.
    last_trace: FrameTrace,
    /// Sequence assigned to the current/last non-idle attempt.
    attempt_seq: u64,
    /// Sequence assigned to the last committed transaction.
    commit_seq: u64,
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
            model_generation: 0,
            last_frame: None,
            spare_slots: RecycledSlots::default(),
            metrics: None,
            pending: PendingWork::default(),
            last_strategy: None,
            last_effective_strategy: None,
            last_work_flags: WorkFlags::empty(),
            last_trace: FrameTrace::default(),
            attempt_seq: 0,
            commit_seq: 0,
        }
    }

    /// Grid-wide attribution for the last `render_pending`. Its verdict is
    /// `None` before the first paint.
    pub fn last_trace(&self) -> FrameTrace {
        self.last_trace
    }

    /// Enable or disable structured frame diagnostics (dev builds only).
    /// Disabling clears the retained snapshot; `frame_diagnostics()`
    /// returns `None` until an enabled attempt completes.
    #[cfg(feature = "dev-diagnostics")]
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        self.grid.renderer.set_diag_enabled(enabled);
    }

    /// Last completed attempt's structured diagnostics, or `None` when
    /// capture is disabled or no enabled attempt has completed. Dev
    /// builds only.
    #[cfg(feature = "dev-diagnostics")]
    pub fn frame_diagnostics(&self) -> Option<FrameDiagnostics> {
        self.grid.renderer.last_diag()
    }

    /// Strategy stamped by the last `render_pending` call.
    /// `None` means that no paint started. The recording pipeline reads it.
    pub fn last_strategy(&self) -> Option<RenderStrategy> {
        self.last_strategy
    }

    /// Diagnostic projection of the work the last `render_pending` acted
    /// upon. Empty before the first paint.
    pub fn last_work_flags(&self) -> WorkFlags {
        self.last_work_flags
    }

    /// Resize both layers in one call. No public per-layer resize, so
    /// callers can't leave the pair half-sized. Self-invalidating: a real
    /// size or DPR change forces the next `render_pending` to `Fresh` — no
    /// caller needs a follow-up `request_repaint()`.
    ///
    /// Takes [`CanvasMetrics`]: the host boundary parses the raw
    /// width/height/DPR once (`CanvasMetrics::new`), and every backend and
    /// geometry walk downstream reads the validated value.
    pub fn resize(&mut self, metrics: CanvasMetrics) {
        if self.metrics == Some(metrics) {
            return;
        }
        self.metrics = Some(metrics);
        self.grid.resize(metrics);
        self.overlay.resize(metrics);
        // A backing-store resize may clear both canvases (Canvas2D), so
        // geometry invalidation must be atomic with the resize itself.
        self.last_frame = None;
        self.pending.mark_geometry();
        self.pending.mark_overlay();
    }

    /// Conservative repaint blanket. Marks geometry so the next
    /// `render_pending` falls to `FullRebuild` — the cheaper `ChangedCells` /
    /// `ScrollBlit` arms gate on geometry being clean. Adds geometry plus
    /// overlay work; it never *adds* content work, which is reserved for
    /// real cell-value changes via `mark_content_dirty`.
    ///
    /// Content and view work already queued is preserved rather than
    /// cleared. Dropping it here would strand an edit that arrived earlier
    /// in the same tick: the escalated `Fresh` frame would rebuild geometry
    /// but lose the content intent that the same Fresh transaction must
    /// subsume.
    ///
    /// `last_frame` is deliberately preserved (see `set_model`'s matching
    /// comment): the geometry work marked below already forces `Fresh`, so
    /// keeping the old committed frame only keeps query geometry coherent
    /// with the old pixels until that Fresh paint lands.
    pub fn request_repaint(&mut self) {
        self.pending.mark_geometry();
        self.pending.mark_overlay();
    }

    /// Bulk-push every overlay primitive in one comparison. The per-field
    /// setters each mark overlay work independently; folding them into one
    /// pass lets the Leptos host's per-frame reactive memo cost a single
    /// mark instead of four.
    pub fn set_overlays(&mut self, overlays: RenderOverlays) {
        if self.decos.set_overlays(overlays) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_extend_to(&mut self, target: Option<AutofillTarget>) {
        if self.decos.set_extend_to(target) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_clipboard(&mut self, area: Option<SheetArea>) {
        if self.decos.set_clipboard(area) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_point_range(&mut self, range: Option<RCRange>) {
        if self.decos.set_point_range(range) {
            self.pending.mark_overlay();
        }
    }

    pub fn set_formula_refs(&mut self, refs: Vec<FormulaRef>) {
        if self.decos.set_formula_refs(refs) {
            self.pending.mark_overlay();
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
        self.pending.mark_overlay();
        id
    }

    /// Remove a custom decoration. Removal is explicit — a layer whose
    /// consumer handle was dropped still participates in the paint and hit
    /// loops (as a no-op) until removed here. Marks overlay work only when
    /// the id was found, so a stale-id call cannot trigger a repaint.
    pub fn remove_decoration(&mut self, id: DecorationId) -> bool {
        let removed = self.decos.remove_custom(id);
        if removed {
            self.pending.mark_overlay();
        }
        removed
    }

    /// Push a theme. Value-compares against `self.theme` and, on change,
    /// marks both layers dirty. `Chrome::classify` rejects a theme-mismatched
    /// frame itself, so the next paint reaches `Fresh` through the
    /// classifier's verdict — no out-of-band `last_frame` drop needed here.
    ///
    /// Deliberately does *not* invalidate the grid paint cache (Stage 6,
    /// Gate A): since the only route out of that classifier rejection is a
    /// `Fresh` walk, and `Layer::paint_grid_fresh` invalidates after its
    /// grid prepares and before its first draw, an eager call here is a
    /// second, redundant painter state transition. Leaving it out also stops
    /// a *held* theme Fresh from touching the painter at all. Cell repaint
    /// coverage does not depend on it either way: `invalidate_paint_cache`
    /// only resets painter ctx state, and a Fresh candidate forces
    /// `RepaintPlan::Full` without consulting the content-keyed fingerprint
    /// tree during full-grid preparation.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != *self.theme {
            self.theme = Rc::new(theme);
            self.pending.mark_geometry();
            self.pending.mark_overlay();
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
        // Wrapping: correctness never depends on uniqueness after a wrap
        // (see the field doc) — this exists to classify an ordinary model
        // replacement, not to gate repaint.
        self.model_generation = self.model_generation.wrapping_add(1);
        // `last_frame` is deliberately preserved (not dropped) here: the
        // geometry + all-content + overlay work marked below already forces
        // the next paint to `Fresh` regardless of `Chrome::classify`'s
        // verdict, so retaining the old committed frame only keeps query
        // geometry (`hit_test`, `cell_rect`, ...) coherent with the old
        // pixels for the window between this call and that Fresh paint —
        // including if the new model's scalar capture temporarily fails.
        // The one setter that *discards* queued work instead of adding to
        // it: row-scoped work recorded against the outgoing model names
        // nothing in the incoming one. Replaced wholesale by the
        // worst-case value, which subsumes anything the old work could
        // have asked for.
        self.pending = PendingWork::default();
        self.pending.mark_geometry();
        self.pending.mark_all_content();
        self.pending.mark_overlay();
    }

    /// Mark the overlay dirty. Selection, autofill, formula-ref, and
    /// clipboard signals funnel through here; grid escalation on scroll /
    /// freeze / sheet / size change is owned by `render_pending` via
    /// `Chrome::classify`, not duplicated at the callsite.
    pub fn request_overlay_repaint(&mut self) {
        self.pending.mark_overlay();
    }

    /// Typed cell-content-changed signal. Marks all visible content dirty so
    /// the next `render_pending` refetches its values from the model via the
    /// grid-wide `SlotsReuse` arm —
    /// fixes the recalc bug where a formula dependent on an edited
    /// cell silently kept painting the stale cached value.
    pub fn mark_content_dirty(&mut self) {
        self.pending.mark_all_content();
    }

    /// Row-scoped `mark_content_dirty`: also names the damaged rows so
    /// `plan_frame` can clip the repaint to full-width bands. All escalation
    /// (cross-sheet rows, span-count cap, or meeting all-content work)
    /// belongs to `ContentWork`'s merge table, not to this callsite.
    ///
    /// Row precision chooses the `Damage` strategy; when that strategy is
    /// ineligible, planning widens the work to `AllContent`.
    pub fn mark_rows_damaged(&mut self, sheet: u32, span: RowSpan) {
        self.pending.mark_rows(sheet, span);
    }

    /// The view moved: scroll, selection, active cell, or sheet. Marks view
    /// plus overlay atomically — a view change always repositions overlay
    /// primitives, and splitting the two would let a caller queue movement
    /// that never repaints the selection rectangle.
    ///
    /// Intent only. Whether the movement shifts pixels (`ScrollBlit`), stays
    /// inside the painted frame (`OverlayOnly`), or needs a rebuild (`FullRebuild`) is
    /// `plan_frame`'s geometric verdict, not the caller's.
    pub fn view_changed(&mut self) {
        self.pending.mark_view();
        self.pending.mark_overlay();
    }

    pub fn canvas_size(&self) -> CanvasSize {
        self.metrics.unwrap_or_else(CanvasMetrics::unresized).size()
    }

    /// Validated canvas metrics for the committed configuration. Before the
    /// first `resize` this is the zero-size, DPR-1.0 default — the same pair
    /// the renderer assumed before the type carried an invariant.
    pub fn metrics(&self) -> CanvasMetrics {
        self.metrics.unwrap_or_else(CanvasMetrics::unresized)
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
    // emitted by the most recent `render_pending`. Before the first paint
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
        let (canvas_w, canvas_h) = frame.canvas_size().to_logical_extent();
        // The frame's own canvas size, not `self.metrics()` — a resize between
        // the last paint and this query must not be mixed into a snapshot answer.
        Some(PixelRect {
            top_left,
            width: (canvas_w - top_left.x).max(0),
            height: (canvas_h - top_left.y).max(0),
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
            |id| crate::geometry::slot::row_height(model, view.sheet, id).extent(),
        )?;
        let left = origin_showing(
            column,
            view.left_column,
            frame.pane_set.cols.frozen_count(),
            pane.width,
            |id| crate::geometry::slot::col_width(model, view.sheet, id).extent(),
        )?;
        ((top, left) != (view.top_row, view.left_column)).then_some((top, left))
    }

    /// Auto-fit width for `col`: widest formatted value across the
    /// `[first_row, last_row]` used-row span, plus padding.
    ///
    /// `Ok(None)` when no scanned cell in `col` has text — nothing to fit to.
    /// `Err(AutoFitError)` when there is no model, or a model read fails: a
    /// host must not treat a failed read as "no content". Pure measurement —
    /// the consumer applies the returned extent.
    pub fn fit_column_width(
        &self,
        col: i32,
        first_row: i32,
        last_row: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        let model = self.model.as_deref().ok_or(AutoFitError::NoModel)?;
        let metrics = self.grid.surface.painter();
        crate::autofit::fit_width(model, metrics, col, first_row, last_row)
    }

    /// Auto-fit height for `row`: tallest font across the `[first_col,
    /// last_col]` used-column span, plus padding. Same
    /// [`AutoFitError`] semantics as `fit_column_width`.
    pub fn fit_row_height(
        &self,
        row: i32,
        first_col: i32,
        last_col: i32,
    ) -> Result<Option<f64>, AutoFitError> {
        let model = self.model.as_deref().ok_or(AutoFitError::NoModel)?;
        let metrics = self.grid.surface.painter();
        crate::autofit::fit_height(model, metrics, row, first_col, last_col)
    }

    pub fn autofill_handle(&self) -> Option<Point> {
        self.last_frame
            .as_ref()?
            .autofill_handle(self.decos.selection().selection_range?)
    }

    /// Paint whichever layers are dirty. Classifies the attempt's geometric
    /// delta via `Chrome::classify`, plans it via `plan_frame` into one
    /// closed `FramePlan`, then dispatches on `plan.grid` into one of five
    /// named strategies: `OverlayOnly`, `ScrollBlit`, `DamagedRows`, `ChangedCells`,
    /// and `FullRebuild`. The `match` is exhaustive — adding a `GridWork` variant
    /// breaks the build here by design.
    pub fn render_pending(&mut self) -> PaintResult {
        // Model-absent -> return *before* taking. Work queued before the
        // first model push describes cells nothing can paint yet; taking it
        // here would drop it silently and leave the first real frame
        // painting stale values.
        if self.model.is_none() {
            return PaintResult::Idle;
        }
        let work = std::mem::take(&mut self.pending);
        if work.is_empty() {
            // Nothing taken, model never taken — nothing to restore.
            return PaintResult::Idle;
        }
        self.attempt_seq = self.attempt_seq.wrapping_add(1);
        // Lift the model out so the paint methods can take `&mut self`
        // without overlapping the model borrow. The `is_none` guard above
        // makes the `else` unreachable, but `let-else` keeps it panic-free.
        let Some(model) = self.model.take() else {
            return PaintResult::Idle;
        };

        // A capture failure still publishes a trace through `finish_attempt`.
        // Clear renderer-owned attribution before any fallible model reads so
        // a held attempt cannot inherit a grid verdict, fetch counts, or blit
        // fallback details from the previously painted frame.
        self.grid.renderer.reset_trace();
        #[cfg(feature = "dev-diagnostics")]
        self.grid.renderer.diag_reset_capture();

        let model_dyn: &dyn CanvasModel = model.as_ref();

        // Capture-failure and retry contract (Stage 3 global constraints).
        // This runs after the model/pending early exits above but before
        // delta classification, plan construction, Chrome mutation, cache
        // invalidation, paint, or presentation — a failure here can hold
        // the whole attempt having touched none of those. Metrics default to
        // a zero-size canvas at DPR 1.0 before the first `resize`, matching
        // the renderer's own default transform.
        let capture = FrameInputs::capture(
            model_dyn,
            self.metrics(),
            Rc::clone(&self.theme),
            self.model_generation,
        );
        // `FrameInputs::capture` here makes a bridge failure on any scalar
        // read observable and holds the attempt (below), rather than the
        // renderer silently painting a synthetic default. The captured value
        // is this frame's only source of sheet/view/freeze/header state:
        // `plan_frame`, `Chrome` construction, and overlay refresh all
        // consume it instead of re-reading the model.
        let inputs = match capture {
            Ok(inputs) => inputs,
            Err(failure) => {
                let flags = work.flags();
                // Capture failure never reaches a strategy — no
                // candidate geometry, no cache invalidation, no paint — so
                // it routes through `finish_attempt` as a `Held` outcome
                // with nothing to install: `frame: FrameUpdate::Preserve`
                // (nothing was ever taken) and the complete taken `work` as
                // its retry.
                let result = self.finish_attempt(
                    None,
                    flags,
                    None,
                    AttemptOutcome::Held {
                        retry: work,
                        frame: FrameUpdate::Preserve,
                        reason: HoldReason::InputFailure(failure),
                    },
                );
                self.model = Some(model);
                return result;
            }
        };

        let delta = Chrome::classify(
            self.last_frame.as_ref(),
            model_dyn,
            &inputs,
            self.decos.selection().active_cell.as_ref(),
        );
        #[cfg(feature = "dev-diagnostics")]
        let diag_delta = DiagDeltaKind::from(&delta);
        let plan = plan_frame(work, delta, inputs.sheet(), inputs.show_selection());
        // Record the classification facts before dispatch; the renderer
        // fills the rest during prepare/execute.
        #[cfg(feature = "dev-diagnostics")]
        self.grid
            .renderer
            .diag_begin_attempt(diag_delta, plan.rebuild_reason);
        let selected = plan.grid.strategy();
        let work_flags = plan.consumes.flags();
        // The trace was reset before capture so both successful dispatch and
        // the capture-failure path describe this attempt only. `OverlayOnly`
        // legitimately leaves the grid verdict `None` — it never
        // calls the grid renderer.
        // `plan.consumes` is the attempt's taken `PendingWork`, owned by the
        // plan; moving it out here (rather than a second borrow of the
        // pre-take value) is what lets a held arm's `AttemptOutcome` carry it
        // straight back to `finish_attempt`'s merge step. An arm that fully
        // commits does nothing further with it; only an arm that holds
        // constructs its grid-wide retry scope from it.
        let overlay_work = plan.overlay;
        let work = plan.consumes;
        let outcome = match plan.grid {
            GridWork::None => self.render_overlay_only(model_dyn, &inputs, work),
            GridWork::Blit(blit_plan) => {
                self.render_scroll_blit(model_dyn, &inputs, blit_plan, work)
            }
            GridWork::AllContent => self.render_changed_cells(model_dyn, &inputs, work),
            GridWork::Fresh => self.render_full_rebuild(model_dyn, &inputs, work),
            GridWork::Rows { sheet, spans } => {
                self.render_damaged_rows(model_dyn, &inputs, sheet, spans, work)
            }
        };
        // Every strategy helper is total — it returns one `AttemptOutcome`
        // for every planned branch (an absent `last_frame` is an explicit
        // invariant hold that requeues the consumed work, never an `Idle`
        // that drops it), so there is no `Option` to fall through here.
        let overlay_ctx = Some(OverlayContext {
            model: model_dyn,
            inputs: &inputs,
            work: overlay_work,
        });
        let result = self.finish_attempt(Some(selected), work_flags, overlay_ctx, outcome);

        // Restore site for every strategy that reached dispatch. The other
        // restore site is the capture-failure early return above, which
        // returns before any strategy runs.
        self.model = Some(model);
        result
    }

    /// The one function that completes a paint attempt. Every
    /// render preparation and execution helper reduces to an
    /// `AttemptOutcome`; this is the completion boundary that advances or
    /// preserves `last_frame`, refreshes and (conditionally) repaints the
    /// overlay against the frame that will be committed, presents whichever
    /// surfaces actually painted, merges retry work back into
    /// `self.pending`, and publishes
    /// `last_strategy`/`last_effective_strategy`/`last_work_flags`/`last_trace`.
    /// It installs the aggregate cache commit before publishing or presenting
    /// the matching frame.
    ///
    /// `selected` and `work_flags` come from `plan_frame`'s verdict —
    /// known before dispatch, so every outcome (including a `Held` capture
    /// failure, which never reaches a strategy) can still stamp them.
    /// `overlay_ctx` is `None` only for that capture-failure case: a
    /// `Held` outcome never refreshes overlay state regardless, so the
    /// context simply isn't there to consult on that branch.
    fn finish_attempt(
        &mut self,
        selected: Option<RenderStrategy>,
        work_flags: WorkFlags,
        overlay_ctx: Option<OverlayContext<'_>>,
        outcome: AttemptOutcome,
    ) -> PaintResult {
        let (committed, grid_painted, cache_commit, frame, retry, effective, frame_outcome, result) =
            match outcome {
                AttemptOutcome::OverlayCommitted => (
                    true,
                    false,
                    None,
                    FrameUpdate::Preserve,
                    None,
                    Some(RenderStrategy::OverlayOnly),
                    FrameOutcome::Painted,
                    PaintResult::Rendered,
                ),
                AttemptOutcome::GridCommitted {
                    cache_commit,
                    frame,
                    effective,
                } => (
                    true,
                    true,
                    Some(cache_commit),
                    frame,
                    None,
                    Some(effective),
                    FrameOutcome::Painted,
                    PaintResult::Rendered,
                ),
                AttemptOutcome::Held {
                    retry,
                    frame,
                    reason,
                } => (
                    false,
                    false,
                    None,
                    frame,
                    Some(retry),
                    None,
                    reason.frame_outcome(),
                    PaintResult::RetryRequired,
                ),
            };
        // Grid pixels executed exactly when the outcome was `GridCommitted`
        // — every grid strategy returns a cache commit, even a grid-wide
        // fingerprint skip, because it installs the fingerprint tree as the
        // committed truth. Presentation follows the same derivation; the
        // separate `PaintedLayers` field that could contradict the outcome
        // is gone, and the variant shape makes the projection exhaustive.
        // `cache_commit` is `Some` iff `grid_painted`, by construction.

        // 1. install the attempt-owned cache commit, then publish the frame
        //    whose pixels and cache metadata it describes. Held outcomes
        //    carry no commit and therefore touch neither persistent cache nor
        //    frame state beyond their explicit rollback/preserve update.
        if let Some(cache_commit) = cache_commit {
            self.grid.commit_grid_cache(cache_commit);
        }
        self.install_frame(frame);

        // (dev only) overlay-painted facts for the diagnostics snapshot must
        // be captured before the common overlay step consumes overlay_ctx.
        #[cfg(feature = "dev-diagnostics")]
        let overlay_painted = committed
            && overlay_ctx
                .as_ref()
                .is_some_and(|ctx| matches!(ctx.work, OverlayWork::Paint));

        if committed {
            if let Some(ctx) = overlay_ctx {
                // Committed attempts refresh committed selection/
                // active-cell state unconditionally — even an
                // `OverlayWork::Preserve` attempt just repainted the grid
                // with new pixels, so the next frame's `Chrome::classify`
                // must compare against the post-edit hash.
                self.decos.refresh_overlay_state(
                    ctx.model,
                    ctx.inputs.sheet(),
                    &ctx.inputs.view(),
                    ctx.inputs.show_selection(),
                );
                if matches!(ctx.work, OverlayWork::Paint)
                    && let Some(frame) = self.last_frame.as_ref()
                {
                    // Overlay-only uses `FrameUpdate::Preserve`, so this
                    // reads the existing committed `Chrome` exactly as
                    // before; every other strategy reads the frame
                    // `install_frame` just installed above. A missing frame
                    // here would mean that invariant broke; the defensive
                    // fallback is to skip this tick's overlay paint (and its
                    // matching present) rather than panic — the retry
                    // machinery already covers an attempt that never painted.
                    self.overlay.paint_overlay_layer(
                        ctx.model,
                        frame,
                        self.decos.selection(),
                        &self.decos.overlay_slice(),
                        self.decos.custom_layers(),
                    );
                    // 2. present the overlay iff the common overlay step
                    //    painted it.
                    self.overlay.present();
                }
            }
            // 3. present the grid iff grid pixels executed (derived above).
            if grid_painted {
                self.grid.present();
            }
        }
        // Held refreshes nothing and paints/presents no overlay — the
        // branch above is simply never entered.

        // 4. merge retry work into any work raised during the attempt.
        if let Some(retry) = retry {
            self.pending.merge(retry);
        }

        let committed_seq = if committed {
            self.commit_seq = self.commit_seq.wrapping_add(1);
            Some(self.commit_seq)
        } else {
            None
        };

        // 5. Publish last_strategy, last_effective_strategy, last_work_flags, and
        //    last_trace — built once here from plan metadata (`selected`/
        //    `work_flags`), the renderer's own prepared-fetch attribution
        //    and grid verdict (`self.grid.renderer.trace()`), and this
        //    outcome's effective strategy/`FrameOutcome`.
        self.last_strategy = selected;
        self.last_effective_strategy = effective;
        self.last_work_flags = work_flags;
        let mut trace = self.grid.renderer.trace();
        trace.attempt_seq = self.attempt_seq;
        trace.committed_seq = committed_seq;
        trace.strategy = selected;
        trace.effective = effective;
        trace.work = work_flags;
        trace.outcome = frame_outcome;
        self.last_trace = trace;

        // 5b. publish the structured diagnostics snapshot. Read after the
        //     cache commit was installed, so `committed_after` reflects the
        //     committed truth. Resolution comes from `AttemptOutcome`, not
        //     from the presence of a grid cache commit: Overlay-only commits
        //     without one.
        #[cfg(feature = "dev-diagnostics")]
        self.grid.renderer.publish_diag(DiagCompletion {
            attempt_seq: self.attempt_seq,
            selected,
            work: work_flags,
            effective,
            committed_seq,
            outcome: frame_outcome,
            layers: DiagPaintedLayers {
                grid: grid_painted,
                overlay: overlay_painted,
            },
            resolution: if committed {
                DiagCacheResolution::Committed
            } else {
                DiagCacheResolution::HeldForRetry
            },
        });

        // 6. return PaintResult::Rendered or PaintResult::RetryRequired.
        result
    }

    /// Preserve or replace `last_frame`, recycling the outgoing frame's
    /// slot Vecs into `spare_slots` whenever one is actually displaced —
    /// the only consumer of that pool is the next `Fresh` attempt's
    /// `Chrome::build` (see `chrome::recycled_slots`'s module doc). A
    /// `Preserve` update never touches either field: the strategy that
    /// produced it either never took `last_frame` out of `self` to begin
    /// with (an atomically-held `Fresh` attempt, or `Overlay`, which has no
    /// candidate at all) or already resolved Held to an equal-content
    /// replacement value instead (see `FrameUpdate`'s doc).
    fn install_frame(&mut self, update: FrameUpdate) {
        if let FrameUpdate::Replace(new_frame) = update {
            if let Some(old) = self.last_frame.take() {
                self.spare_slots = RecycledSlots::from_pane_set(old.pane_set);
            }
            self.last_frame = Some(new_frame);
        }
    }

    /// Overlay-only fast path: reuses the committed frame verbatim with no
    /// grid touch at all. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. Preparation/execution
    /// helper only — `finish_attempt` does the actual overlay
    /// refresh/paint/present, reading `self.last_frame` as it already
    /// stands (`OverlayCommitted` implies `FrameUpdate::Preserve`). If the
    /// committed-frame precondition is unexpectedly absent, it falls back to
    /// a safe full rebuild in the same attempt.
    fn render_overlay_only(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
    ) -> AttemptOutcome {
        if self.last_frame.is_none() {
            return self.render_full_rebuild(model, inputs, work);
        }
        AttemptOutcome::OverlayCommitted
    }

    /// Scroll-blit fast path. `plan_frame` already filtered no-op scrolls and
    /// viewport shifts where the kept band can't be reused; we trust the
    /// verdict and the supplied plan.
    ///
    /// Calls `Chrome::prepare_blit` — not the immediate-commit
    /// `Chrome::next_blit` — because a successfully-built candidate can
    /// still fail atomically: `paint_grid_blit`'s strip prefetch runs
    /// *after* `prepare_blit` returns, against the candidate it already
    /// built. Holding the `PreparedBlitFrame` open until that result is
    /// known is what lets the `Held` outcome carry `prepared.rollback()`
    /// instead of restoring from a clone taken up front. `prepare_blit`'s
    /// `FreshFallback` arm is the demote-to-`Fresh` path (e.g. a row-header
    /// digit boundary rejects in-place reuse), delegated to
    /// [`Self::paint_fresh_fallback`] — the same atomic-Fresh mechanics
    /// `render_full_rebuild` uses, since a `FreshFallback`'s geometry and
    /// full-canvas background differ from the committed frame exactly like
    /// an ordinary Fresh rebuild's do.
    fn render_scroll_blit(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: BlitPlan,
        work: PendingWork,
    ) -> AttemptOutcome {
        let Some(prev) = self.last_frame.take() else {
            return self.render_full_rebuild(model, inputs, work);
        };
        match Chrome::prepare_blit(prev, model, inputs, &plan) {
            PreparedBlitOutcome::Ready(prepared) => {
                match self.grid.paint_grid_blit(model, prepared.frame(), &plan) {
                    GridPaintOutcome::Committed(cache_commit) => AttemptOutcome::GridCommitted {
                        cache_commit,
                        frame: FrameUpdate::Replace(prepared.commit()),
                        effective: RenderStrategy::ScrollBlit,
                    },
                    GridPaintOutcome::Held => {
                        // Whole-frame hold: the preflight aborted before a pixel
                        // shifted, so nothing at all was committed. `rollback`
                        // moves `prev`'s untouched pieces back out of the
                        // now-discarded candidate — no clone was ever taken —
                        // so this is exactly what `last_frame` held before the
                        // attempt; the entire attempt (including the overlay
                        // mark, which never painted) comes back via `retry`.
                        AttemptOutcome::Held {
                            retry: retry_grid_wide(work),
                            frame: FrameUpdate::Replace(prepared.rollback()),
                            reason: HoldReason::BridgeFailure,
                        }
                    }
                }
            }
            PreparedBlitOutcome::FreshFallback(prev) => {
                #[cfg(feature = "dev-diagnostics")]
                self.grid.renderer.diag_blit(
                    &plan,
                    DiagBlitResultTag::FreshFallback,
                    None,
                    None,
                    prev.grid_layout(),
                );
                self.paint_fresh_fallback(model, inputs, work, prev)
            }
        }
    }

    /// Shared full-rebuild construction tail for `render_full_rebuild` and
    /// `render_scroll_blit`'s `FreshFallback` sub-path. This function builds a
    /// `Fresh`-kind candidate from `self.spare_slots` (never touching
    /// `self.last_frame` — see the module's Stage 4 design doc's Fresh
    /// recipe) and paints the grid atomically. Returns the candidate and its
    /// grid-wide held verdict. The caller still owns the held-vs-committed
    /// `FrameUpdate`/pool-recycling decision, because the two callers
    /// differ in what (if anything) they must hand back on Held: ordinary
    /// Fresh never took `last_frame` at all, but `FreshFallback` already
    /// took it for the original blit attempt and holds `prev` locally.
    /// Returns `Err(recycled)` — the drained pool, for the caller to stash
    /// back into `self.spare_slots` — when a row-height or column-width read
    /// failed transiently during `Chrome::build`, before any paint. See
    /// `FreshBuild::Held`'s doc.
    fn build_and_paint_fresh(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
    ) -> Result<(Chrome, GridPaintOutcome), RecycledSlots> {
        let spare = std::mem::take(&mut self.spare_slots);
        let frame = match Chrome::build(model, inputs, spare) {
            FreshBuild::Ready(frame) => frame,
            FreshBuild::Held(recycled) => return Err(recycled),
        };
        // `paint_grid_fresh` prepares the whole grid before touching the
        // painter at all (not even the cache invalidation or background
        // fill), so a held attempt is a true no-op here — see its doc.
        let cache_commit = self.grid.paint_grid_fresh(model, &frame);
        Ok((frame, cache_commit))
    }

    /// `render_scroll_blit`'s `FreshFallback` arm: `prepare_blit` rejected
    /// in-place reuse and handed `prev` back whole (never partially
    /// consumed — see `try_blit_reuse`'s doc), so it is still available
    /// here as an ordinary owned value, not something sitting in
    /// `self.last_frame` to roll back out of. Builds and paints exactly
    /// like an ordinary Fresh attempt; the only difference is what a held
    /// or committed outcome does with `prev` (see the two arms below).
    fn paint_fresh_fallback(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
        prev: Chrome,
    ) -> AttemptOutcome {
        let (frame, paint) = match self.build_and_paint_fresh(model, inputs) {
            Ok(ok) => ok,
            Err(recycled) => {
                // Geometry held before any paint. `prev` was already taken
                // out of `self.last_frame` for the original blit attempt, so
                // it must be handed back explicitly via `Replace(prev)`;
                // `prev`'s own vecs are still inside it, so only the failed
                // candidate's drained pool needs stashing for the retry.
                self.spare_slots = recycled;
                return AttemptOutcome::Held {
                    retry: retry_grid_wide(work),
                    frame: FrameUpdate::Replace(prev),
                    reason: HoldReason::BridgeFailure,
                };
            }
        };

        let cache_commit = match paint {
            GridPaintOutcome::Committed(cache_commit) => cache_commit,
            GridPaintOutcome::Held => {
                // Atomic hold: park the failed candidate's own vecs for reuse
                // and hand `prev` back explicitly — unlike an ordinary Fresh
                // hold, `prev` isn't sitting in `self.last_frame` for
                // `finish_attempt` to simply leave alone; it was already taken
                // out for the original blit attempt above.
                self.spare_slots = RecycledSlots::from_pane_set(frame.pane_set);
                return AttemptOutcome::Held {
                    retry: retry_grid_wide(work),
                    frame: FrameUpdate::Replace(prev),
                    reason: HoldReason::BridgeFailure,
                };
            }
        };

        // `prev`'s own vecs are about to be displaced by `frame`; recycle
        // them into the pool exactly like an ordinary Fresh commit does in
        // `install_frame` — `self.last_frame` was already emptied for the
        // original blit attempt, so `finish_attempt`'s own recycle step
        // would otherwise find nothing to fold in.
        self.spare_slots = RecycledSlots::from_pane_set(prev.pane_set);
        AttemptOutcome::GridCommitted {
            cache_commit,
            frame: FrameUpdate::Replace(frame),
            effective: RenderStrategy::FullRebuild,
        }
    }

    /// Damaged-rows strategy: slot vectors survive as in `ChangedCells`.
    /// prior grid pixels stay, and only damaged bands refetch and repaint.
    /// Preparation collects every required strip before execution; a bridge
    /// failure leaves committed `GridCache` buffers, fingerprints, layout,
    /// and pixels untouched instead of partially splicing them.
    fn render_damaged_rows(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        _sheet: u32,
        spans: Vec<RowSpan>,
        work: PendingWork,
    ) -> AttemptOutcome {
        let Some(prev) = self.last_frame.take() else {
            return self.render_full_rebuild(model, inputs, work);
        };
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);
        match self.grid.paint_grid_damage(model, &frame, &spans) {
            GridPaintOutcome::Committed(cache_commit) => AttemptOutcome::GridCommitted {
                cache_commit,
                frame: FrameUpdate::Replace(frame),
                effective: RenderStrategy::DamagedRows,
            },
            GridPaintOutcome::Held => AttemptOutcome::Held {
                retry: retry_grid_wide(work),
                frame: FrameUpdate::Replace(frame),
                reason: HoldReason::BridgeFailure,
            },
        }
    }

    /// Changed-cells strategy: the previous slot vectors survive.
    /// Grid preparation refetches visible content and fingerprint-skips when
    /// it matches the committed `GridCache`. No eager cache invalidation is
    /// needed: the candidate cache commit is installed only after every
    /// segment prepares and the grid transaction executes successfully.
    fn render_changed_cells(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
    ) -> AttemptOutcome {
        let Some(prev) = self.last_frame.take() else {
            return self.render_full_rebuild(model, inputs, work);
        };
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);
        match self.grid.paint_grid(model, &frame) {
            GridPaintOutcome::Committed(cache_commit) => AttemptOutcome::GridCommitted {
                cache_commit,
                frame: FrameUpdate::Replace(frame),
                effective: RenderStrategy::ChangedCells,
            },
            GridPaintOutcome::Held => AttemptOutcome::Held {
                retry: retry_grid_wide(work),
                frame: FrameUpdate::Replace(frame),
                reason: HoldReason::BridgeFailure,
            },
        }
    }

    /// Full grid repaint. Slot vecs walked fresh from the model; the new
    /// vecs make any cross-frame fingerprint compare meaningless, so the
    /// whole grid repaints. Selected when slot vecs diverged or no prior frame.
    ///
    /// Builds via [`Self::build_and_paint_fresh`] (`Chrome::build`, not
    /// `Chrome::next(.., FramePath::Fresh)`): the latter derives its
    /// `RecycledSlots` from `prev` inline, draining `prev`'s pane_set
    /// before this attempt's paint is known to succeed. Building from
    /// `self.spare_slots` instead — a pool independent of
    /// `self.last_frame` — means `prev` is never touched during the build
    /// at all, so a held attempt leaves `self.last_frame` exactly as it
    /// was (see `FrameUpdate::Preserve`'s doc); only a committed attempt
    /// folds the outgoing `prev` into the pool, via `install_frame`.
    fn render_full_rebuild(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
    ) -> AttemptOutcome {
        let (frame, paint) = match self.build_and_paint_fresh(model, inputs) {
            Ok(ok) => ok,
            Err(recycled) => {
                // Geometry held before any paint: the drained pool goes back
                // to `spare_slots`, and `last_frame` is left completely
                // untouched — it was never taken (see
                // `build_and_paint_fresh`), so `prev` (or `None`, on a held
                // first frame) is exactly what `finish_attempt` will see.
                self.spare_slots = recycled;
                return AttemptOutcome::Held {
                    retry: retry_grid_wide(work),
                    frame: FrameUpdate::Preserve,
                    reason: HoldReason::BridgeFailure,
                };
            }
        };

        let cache_commit = match paint {
            GridPaintOutcome::Committed(cache_commit) => cache_commit,
            GridPaintOutcome::Held => {
                // Atomic hold: give the failed candidate's own vecs back to the
                // pool and leave `last_frame` completely untouched — it was
                // never taken (see `build_and_paint_fresh`), so `prev` (or
                // `None`, on a held first frame) is exactly what
                // `finish_attempt` will still see.
                self.spare_slots = RecycledSlots::from_pane_set(frame.pane_set);
                return AttemptOutcome::Held {
                    retry: retry_grid_wide(work),
                    frame: FrameUpdate::Preserve,
                    reason: HoldReason::BridgeFailure,
                };
            }
        };

        AttemptOutcome::GridCommitted {
            cache_commit,
            frame: FrameUpdate::Replace(frame),
            effective: RenderStrategy::FullRebuild,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameInputFailure, FrameOutcome, HoldReason, origin_showing, retry_grid_wide};
    use crate::frame::work::{ContentWork, PendingWork, RowSpan};

    /// Uniform rows, so `extent / 20` is how many fit and every expectation
    /// below is arithmetic a reader can redo in their head.
    fn rows_20(_id: i32) -> Option<i32> {
        Some(20)
    }

    #[test]
    fn origin_showing_does_not_overflow_on_large_valid_extents() {
        assert_eq!(origin_showing(3, 1, 0, 100, |_| Some(i32::MAX)), Some(3));
    }

    #[test]
    fn bridge_retry_widens_content_and_preserves_other_intent() {
        let mut work = PendingWork::default();
        work.mark_rows(7, RowSpan::new(2, 4));
        work.mark_view();
        work.mark_overlay();

        let retry = retry_grid_wide(work);

        assert_eq!(*retry.content(), ContentWork::All);
        assert!(retry.has_view());
        assert!(retry.has_overlay());
    }

    /// `HoldReason` is the private authority for held-frame outcomes; the
    /// public `FrameOutcome` and wire names are derived from it, so a held
    /// cause and a committed report can never contradict each other.
    #[test]
    fn hold_reason_projects_to_the_public_frame_outcome() {
        assert_eq!(
            HoldReason::InputFailure(FrameInputFailure::SelectedSheet).frame_outcome(),
            FrameOutcome::HeldOnInputFailure(FrameInputFailure::SelectedSheet)
        );
        assert_eq!(
            HoldReason::BridgeFailure.frame_outcome(),
            FrameOutcome::HeldOnBridgeFailure
        );
    }

    #[test]
    fn stays_put_when_there_is_nothing_to_scroll() {
        // A collapsed axis has nowhere to put the target.
        assert_eq!(origin_showing(50, 7, 0, 0, rows_20), Some(7));
        assert_eq!(origin_showing(50, 7, 0, -100, rows_20), Some(7));
        // A frozen target is painted whatever the scrollable band shows.
        assert_eq!(origin_showing(2, 7, 3, 500, rows_20), Some(7));
    }

    #[test]
    fn flushes_against_the_near_edge_when_scrolled_past() {
        assert_eq!(origin_showing(5, 20, 0, 500, rows_20), Some(5));
    }

    /// The trailing `max` earns its keep here: the walk finds row 8 as the
    /// earliest origin that fits, but the band already sits at 10 and already
    /// shows row 12 — scrolling back to 8 would be visible, pointless motion.
    #[test]
    fn leaves_an_already_visible_target_alone() {
        assert_eq!(origin_showing(12, 10, 0, 100, rows_20), Some(10));
    }

    /// A target past the far edge pulls the origin forward — to the *smallest*
    /// origin that still shows the target whole, not merely to the target.
    #[test]
    fn walks_back_to_the_smallest_origin_that_shows_the_target() {
        // Five 20 px rows fill 100, so 26..=30 is the earliest band showing 30.
        // An implementation that stopped after one step would answer 29.
        assert_eq!(origin_showing(30, 2, 0, 100, rows_20), Some(26));
    }

    /// Rows of 8/19/30/41/52 px on a five-row cycle, so a walk that assumed a
    /// uniform height cannot land on the right origin by symmetry.
    fn rows_uneven(id: i32) -> Option<i32> {
        Some(8 + id.rem_euclid(5) * 11)
    }

    /// The walk accumulates real heights: rows 30 (8 px) and 29 (52 px) fill 60
    /// of the 100 px band, and taking row 28 (41 px) too would overflow it.
    #[test]
    fn walk_sums_actual_row_heights_rather_than_assuming_uniform_rows() {
        assert_eq!(origin_showing(30, 2, 0, 100, rows_uneven), Some(29));
    }
}
