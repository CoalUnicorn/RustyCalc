//! Single-axis scroll blit fast-path: detection, plan construction, and
//! Chrome reuse around a `Painter::blit` shift.
//!
//! `Chrome::classify` (mod.rs) screens the disqualifiers (sheet / freeze /
//! canvas / DPR / theme / model generation / headers / active-cell mismatch,
//! two-axis scroll); on a viable single-axis scroll it delegates to
//! `try_blit_rows` / `try_blit_cols` here, which return a `BlitPlan` if the
//! geometry checks out. The orchestrator then calls `Chrome::next_blit(..,
//! &plan)`, which routes through `try_blit_reuse` to construct the next
//! frame in place — kept band carries forward, only the strip hits the
//! model.

use std::rc::Rc;

use crate::CanvasModel;
use crate::frame::{BlitPlan, FrameInputs};
use crate::geometry::CanvasMetrics;
use crate::theme::CanvasTheme;

use super::pane_set::ScrollAxisSlots;
use super::{BlitOutcome, Chrome, FrameKindTag, FramePath};

mod rebuild;
mod reuse;

use self::reuse::try_blit_reuse;
pub(crate) use reuse::{try_blit_cols, try_blit_rows};

/// Result of [`try_blit_reuse`]: either a reversible in-place candidate, or
/// `prev` handed back whole so the caller rebuilds `Fresh`.
///
/// `FreshFallback` is a normal scheduler outcome — the row-header digit
/// boundary, or a cross-axis model anomaly — not a fault, so it is a named
/// variant rather than the `Err` arm of a `Result`. Callers therefore cannot
/// mistake it for a technical failure, and no `?` can propagate it by
/// accident. The public immediate-commit wrapper
/// ([`BlitOutcome`](crate::chrome::BlitOutcome)) reports the same two cases.
// Large by value on both arms on purpose: the reject arm gives `prev` back
// with zero copies, and boxing would add a heap allocation to the
// steady-state blit path.
#[allow(clippy::large_enum_variant)]
pub(crate) enum PreparedBlitOutcome {
    /// In-place reuse succeeded; the candidate is reversible until the
    /// caller commits or rolls back.
    Ready(PreparedBlitFrame),
    /// In-place reuse rejected; `Chrome` is handed back intact for a
    /// `Fresh` rebuild.
    FreshFallback(Chrome),
}

/// Reversible construction of a scroll-blit's next-frame `Chrome`. Wraps a
/// successfully-built candidate together with the exact pieces of `prev`
/// [`try_blit_reuse`] replaced to build it, so a caller that only learns
/// about a failure *after* construction succeeded — `paint_grid_blit`'s
/// strip-prefetch bridge check, run against the already-built candidate —
/// can reconstruct `prev` by moving those pieces back out of `candidate`,
/// rather than keeping a full clone of `prev` around "just in case".
///
/// `Chrome::next_blit` is the immediate-commit convenience wrapper most
/// callers want (and the only shape `BlitOutcome`'s two variants need).
/// `Chrome::prepare_blit` hands back this type instead, for the one caller
/// (`Orchestrator::render_scroll_blit`) that must hold the
/// commit-or-rollback decision open until it knows whether the paint that
/// follows actually succeeded.
pub(crate) struct PreparedBlitFrame {
    candidate: Chrome,
    rollback: BlitRollback,
}

impl PreparedBlitFrame {
    /// Borrow the candidate frame — e.g. to hand to `paint_grid_blit` before
    /// deciding whether to commit or roll back.
    pub(crate) fn frame(&self) -> &Chrome {
        &self.candidate
    }

    pub(crate) fn commit(self) -> Chrome {
        self.candidate
    }

    /// Reconstruct `prev` by moving the frozen bands and the untouched
    /// cross-axis scroll Vec back out of `candidate` (via
    /// [`PaneSet::swap_scroll_axis`]), and swapping in `rollback`'s saved
    /// originals for every field `candidate` replaced instead of carrying
    /// forward. No field is cloned: everything `candidate` owns either
    /// becomes part of the reconstructed `Chrome` or is dropped in favor of
    /// a `rollback` field that was itself moved — never cloned — out of
    /// `prev` before `try_blit_reuse` built `candidate` in the first place.
    pub(crate) fn rollback(self) -> Chrome {
        let PreparedBlitFrame {
            candidate,
            rollback,
        } = self;
        let BlitRollback {
            scroll,
            row_header_labels,
            col_header_labels,
            theme,
            metrics,
            model_generation,
            show_row_headers,
            show_col_headers,
            kind,
        } = rollback;
        // `theme`/`metrics`/`model_generation`/`show_row_headers`/
        // `show_col_headers`/`kind` all came from `inputs`/`FrameKindTag::Blitted`
        // when `candidate` was built, not from `prev` — dropped here in
        // favor of `rollback`'s saved originals, bound above.
        let Chrome {
            sheet,
            pane_set,
            row_header_thickness,
            col_header_thickness,
            cell_origin,
            ..
        } = candidate;
        Chrome {
            sheet,
            pane_set: pane_set.swap_scroll_axis(scroll, row_header_labels, col_header_labels),
            row_header_thickness,
            col_header_thickness,
            cell_origin,
            metrics,
            theme,
            model_generation,
            show_row_headers,
            show_col_headers,
            kind,
        }
    }
}

/// The exact fields `try_blit_reuse` replaces when it builds `candidate`
/// from `prev`, saved so [`PreparedBlitFrame::rollback`] can restore them
/// without re-deriving anything from `inputs` or the model. `sheet`,
/// `col_header_thickness`, `cell_origin`, and `row_header_thickness` are
/// deliberately not here: `try_blit_reuse` always either copies them from
/// `prev` unchanged or proves them equal to
/// `prev`'s value — via its own row-header-thickness gate, or via
/// `Chrome::classify`'s canvas-size/etc. hard breaks that must pass before
/// `try_blit_reuse` is ever called — before `candidate` is built, so
/// reading them back off `candidate` at rollback time is already exact;
/// storing a second copy here would be redundant, not more correct.
struct BlitRollback {
    /// The scrolled axis's *original* slot Vec — the one value `prev` owned
    /// that `candidate` does not carry forward at all (its replacement is
    /// freshly rebuilt from the model, not moved).
    scroll: ScrollAxisSlots,
    row_header_labels: Vec<String>,
    col_header_labels: Vec<String>,
    theme: Rc<CanvasTheme>,
    metrics: CanvasMetrics,
    model_generation: u64,
    show_row_headers: bool,
    show_col_headers: bool,
    kind: FrameKindTag,
}

impl Chrome {
    /// Prepare the blit fast-path's next-frame candidate without committing:
    /// [`PreparedBlitOutcome::Ready`] on successful in-place reuse,
    /// [`PreparedBlitOutcome::FreshFallback`] carrying `prev` whole on reject
    /// (see `try_blit_reuse`'s doc for both cases). `Chrome::next_blit` is the
    /// immediate-commit wrapper built on top of this for callers that don't
    /// need to hold the decision open; `Orchestrator::render_scroll_blit`
    /// calls this directly instead, so it can call
    /// `PreparedBlitFrame::rollback` if the paint that follows a successful
    /// `Ready` still fails a bulk bridge read. `pub(crate)`: an execution
    /// detail of the render pipeline, not consumer-facing API.
    pub(crate) fn prepare_blit(
        prev: Chrome,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: &BlitPlan,
    ) -> PreparedBlitOutcome {
        try_blit_reuse(prev, model, inputs, plan)
    }

    /// Build the next-frame `Chrome` for the blit fast-path, returning a typed
    /// [`BlitOutcome`] rather than a `Chrome` with an open `FrameKindTag`.
    ///
    /// Qualification passed (`Chrome::classify` returned `FrameDelta::Scroll`),
    /// but in-place reuse may still reject — e.g. the row-header digit boundary at 99 -> 100,
    /// where `row_header_thickness` widens and the cross-axis cell-area origin
    /// shifts. `try_blit_reuse` hands `prev` back
    /// (`PreparedBlitOutcome::FreshFallback`) on reject, and we rebuild
    /// `Fresh`. The two outcomes map straight to the two `BlitOutcome` arms at
    /// the decision point, so no caller has to assert an impossible
    /// `SlotsReused` away.
    ///
    /// Implemented through [`Self::prepare_blit`] — the same internal
    /// candidate builder `Orchestrator::render_scroll_blit` uses — with an
    /// immediate `.commit()`: there is no second blit construction algorithm,
    /// only a second (non-atomic) way to consume the first one's result.
    pub fn next_blit(
        prev: Option<Chrome>,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: &BlitPlan,
    ) -> BlitOutcome {
        let Some(prev) = prev else {
            return BlitOutcome::FreshFallback(Self::next(None, model, inputs, FramePath::Fresh));
        };
        match Self::prepare_blit(prev, model, inputs, plan) {
            PreparedBlitOutcome::Ready(prepared) => BlitOutcome::Blitted(prepared.commit()),
            PreparedBlitOutcome::FreshFallback(prev) => {
                BlitOutcome::FreshFallback(Self::next(Some(prev), model, inputs, FramePath::Fresh))
            }
        }
    }
}
