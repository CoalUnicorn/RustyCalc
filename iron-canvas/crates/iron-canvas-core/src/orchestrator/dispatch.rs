use std::rc::Rc;

use crate::CanvasModel;
use crate::chrome::Chrome;
use crate::frame::{FrameInputs, GridWork, PaintResult, plan_frame};
use crate::layer::Surface;
use crate::painter::BlitPainter;
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diagnostics::DiagDeltaKind;

use super::Orchestrator;
use super::finish::{AttemptOutcome, FrameUpdate, HoldReason, OverlayContext};

impl<S> Orchestrator<S>
where
    S: Surface,
    S::P: BlitPainter,
{
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
}
