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
use crate::chrome::{Chrome, FramePath, FreshBuild, PreparedBlitOutcome, RecycledSlots};
use crate::decoration::Decorations;
use crate::frame::work::{PendingWork, RowSpan, WorkFlags};
use crate::frame::{
    BlitPlan, FrameInputs, FrameTrace, GridWork, PaintResult, RenderStrategy, plan_frame,
};
use crate::geometry::CanvasMetrics;
use crate::layer::{LayerBase, Surface};
use crate::painter::BlitPainter;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::{DiagBlitResultTag, DiagDeltaKind, FrameDiagnostics};
use crate::renderer::{GridPaintOutcome, GridRenderer, OverlayRenderer};
use crate::theme::CanvasTheme;

use self::finish::{AttemptOutcome, FrameUpdate, HoldReason, OverlayContext, retry_grid_wide};

mod finish;
mod query;
mod setters;

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
