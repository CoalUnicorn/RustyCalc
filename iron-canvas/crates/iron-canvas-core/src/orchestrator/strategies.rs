use crate::CanvasModel;
use crate::chrome::{Chrome, FramePath, FreshBuild, PreparedBlitOutcome, RecycledSlots};
use crate::frame::work::{PendingWork, RowSpan};
use crate::frame::{BlitPlan, FrameInputs, RenderStrategy};
use crate::layer::Surface;
use crate::painter::BlitPainter;
use crate::renderer::GridPaintOutcome;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::DiagBlitResultTag;

use super::Orchestrator;
use super::finish::{AttemptOutcome, FrameUpdate, HoldReason, retry_grid_wide};

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
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
    pub(super) fn render_scroll_blit(
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
    pub(super) fn render_damaged_rows(
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
    pub(super) fn render_changed_cells(
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
    pub(super) fn render_full_rebuild(
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
