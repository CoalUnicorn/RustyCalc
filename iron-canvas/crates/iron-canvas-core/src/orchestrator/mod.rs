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
use crate::chrome::{Chrome, RecycledSlots};
use crate::decoration::Decorations;
use crate::frame::work::{PendingWork, WorkFlags};
use crate::frame::{FrameTrace, RenderStrategy};
use crate::geometry::CanvasMetrics;
use crate::painter::BlitPainter;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::FrameDiagnostics;
use crate::renderer::{GridRenderer, OverlayRenderer};
use crate::surface::{LayerBase, Surface};
use crate::theme::CanvasTheme;

mod dispatch;
mod finish;
mod query;
mod setters;
mod strategies;

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
}
