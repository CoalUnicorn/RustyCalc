use crate::CanvasModel;
use crate::chrome::{Chrome, RecycledSlots};
use crate::frame::work::{PendingWork, WorkFlags};
use crate::frame::{
    FrameInputFailure, FrameInputs, FrameOutcome, OverlayWork, PaintResult, RenderStrategy,
};
use crate::layer::Surface;
use crate::painter::BlitPainter;
use crate::renderer::GridCacheCommit;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::{DiagCacheResolution, DiagCompletion, DiagPaintedLayers};

use super::Orchestrator;

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
pub(super) enum FrameUpdate {
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
pub(super) struct OverlayContext<'a> {
    pub(super) model: &'a dyn CanvasModel,
    pub(super) inputs: &'a FrameInputs,
    pub(super) work: OverlayWork,
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
pub(super) enum AttemptOutcome {
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
pub(super) enum HoldReason {
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
pub(super) fn retry_grid_wide(mut work: PendingWork) -> PendingWork {
    work.mark_all_content();
    work
}

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
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
    pub(super) fn finish_attempt(
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
    pub(super) fn render_overlay_only(
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
}

#[cfg(test)]
mod tests {
    use super::{HoldReason, retry_grid_wide};
    use crate::frame::work::{ContentWork, PendingWork, RowSpan};
    use crate::frame::{FrameInputFailure, FrameOutcome};

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
}
