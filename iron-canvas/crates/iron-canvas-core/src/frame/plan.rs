//! The frame planner: the intent-plus-delta decision that turns one paint
//! attempt into a closed `FramePlan`.
//!
//! `plan_frame` is pure: everything it needs is either inside the taken
//! `PendingWork` and the `FrameDelta`, the current sheet, or the captured
//! selection visibility. `GridWork` is the single authority for the
//! `RenderStrategy` tag.

use serde::{Deserialize, Serialize};

use crate::frame::delta::{BlitPlan, FrameDelta, RebuildReason};
use crate::frame::work::{ContentWork, PendingWork, RowSpan};

/// Data-free strategy tag. Stamped by `render_pending` from
/// [`GridWork::strategy`] — derived, never stored alongside the work — into
/// `Orchestrator.last_strategy` so out-of-engine consumers (the recording
/// pipeline) can attribute each captured frame to a strategy without seeing
/// the plan's inner data (`BlitPlan`, row spans — see `GridWork`). Serializes
/// with snake_case variant names to match the `.icr` JSON-lines schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[must_use = "RenderStrategy records the selected strategy; dropping it skips a recorder frame"]
#[serde(rename_all = "snake_case")]
pub enum RenderStrategy {
    OverlayOnly,
    ScrollBlit,
    ChangedCells,
    FullRebuild,
    DamagedRows,
}

/// What `plan_frame` decided the grid needs this attempt. Each variant
/// carries the payload that its render method needs.
/// These payloads have the same shapes as the former `PaintRegime` values.
/// planning and execution were split into their own closed types.
///
/// `GridWork` alone determines candidate `Chrome` construction exhaustively:
///
/// | `GridWork` | candidate geometry |
/// | --- | --- |
/// | `None` | borrow committed `Chrome` |
/// | `Fresh` | fresh `Chrome` walk |
/// | `AllContent` | slots-reused `Chrome` |
/// | `Rows { .. }` | slots-reused `Chrome` |
/// | `Blit(plan)` | blit-reused `Chrome`, with typed Fresh fallback |
///
/// There is no second stored `CandidateFrame` enum: storing one alongside
/// `GridWork` would admit contradictions such as a `Fresh` candidate paired
/// with `Rows` work.
#[must_use = "GridWork is the grid dispatch verdict; dropping it means the chosen paint_* method never runs"]
pub(crate) enum GridWork {
    /// No grid touch at all — the committed `Chrome` is reused as-is.
    None,
    /// Full rebuild: `FramePath::Fresh` construction, whole grid repainted.
    Fresh,
    /// `FramePath::SlotsReuse` construction; the visible grid refetches and
    /// repaints.
    AllContent,
    /// `FramePath::SlotsReuse` construction; only the named row bands —
    /// on `sheet`, the sheet the content work was originally recorded
    /// against — refetch and repaint via the blit-strip machinery.
    Rows { sheet: u32, spans: Vec<RowSpan> },
    /// `Chrome::next_blit` construction; the kept band ships via
    /// `Painter::blit` and only the plan's repaint strip refetches.
    Blit(BlitPlan),
}

impl GridWork {
    /// The `RenderStrategy` this work selects — the single authority for
    /// the strategy tag. `FramePlan` deliberately stores no separate
    /// strategy field: the mapping is one-to-one (each variant names one
    /// strategy), so a second stored value could contradict the work that
    /// was actually dispatched.
    pub(crate) fn strategy(&self) -> RenderStrategy {
        match self {
            GridWork::None => RenderStrategy::OverlayOnly,
            GridWork::Fresh => RenderStrategy::FullRebuild,
            GridWork::AllContent => RenderStrategy::ChangedCells,
            GridWork::Rows { .. } => RenderStrategy::DamagedRows,
            GridWork::Blit(_) => RenderStrategy::ScrollBlit,
        }
    }
}

/// Whether this attempt must repaint the overlay layer, computed once by
/// `plan_frame` so every execution arm reads the same verdict instead of
/// re-deriving `must_paint_overlay` from `PendingWork` and decoration
/// state. See `plan_frame`'s doc comment for the exact rule per pending-work
/// category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayWork {
    /// Leave the overlay surface exactly as the previous frame left it.
    Preserve,
    Paint,
}

/// The closed output of `plan_frame`: everything `render_pending` needs to
/// dispatch one paint attempt, plus the taken `PendingWork` the plan was
/// built from — owned here so a held/retried arm has it to merge back into
/// `self.pending` without a second, separate borrow of the pre-take value.
pub(crate) struct FramePlan {
    /// The dispatched grid work. Its `strategy()` is stamped into
    /// `Orchestrator.last_strategy` before dispatch; that derived value may
    /// still diverge from what actually painted — see `FrameTrace::effective`'s
    /// doc for the selected-`ScrollBlit`/effective-`FullRebuild` case, which no plan
    /// field encodes.
    pub(crate) grid: GridWork,
    pub(crate) overlay: OverlayWork,
    /// The attempt's taken `PendingWork`, owned by the plan so a held
    /// execution arm (`render_scroll_blit`'s whole-frame hold) can merge
    /// it back into `self.pending` verbatim.
    pub(crate) consumes: PendingWork,
    /// Which hard break or scroll incompatibility fired, when `grid` is
    /// `Fresh` because of one. Read by the dev-diagnostics capture (the
    /// only reader) after `plan_frame`; unread in feature-off builds.
    #[cfg_attr(not(feature = "dev-diagnostics"), allow(dead_code))]
    pub(crate) rebuild_reason: Option<RebuildReason>,
}

/// Build the plan for one paint attempt from its taken `PendingWork` and the
/// `FrameDelta` `Chrome::classify` returned for it. Pure: everything it
/// needs is either already inside `work`/`delta`, the current sheet (used
/// only to check whether row-content work was recorded against the sheet
/// still on screen — a `Stable`/`Scroll` delta already proves that sheet
/// agrees with the committed frame's, so no `last_frame` access is needed
/// here), or — the one additional overlay-policy input — `show_selection`,
/// the frame's captured selection visibility.
///
/// Implements the Stage 3 planner table, cheapest arm first:
///
/// | attempted work and live delta | strategy / grid work |
/// | --- | --- |
/// | overlay/view only, `Stable` | `OverlayOnly` / `GridWork::None` |
/// | overlay/view only, `Scroll(plan)` | `ScrollBlit` / `GridWork::Blit(plan)` |
/// | overlay/view only, `Rebuild` | `FullRebuild` / `GridWork::Fresh` |
/// | row content, optional view, `Stable`, sheet matches | `DamagedRows` / `GridWork::Rows` |
/// | row content, optional view, `Stable`, sheet differs | `ChangedCells` / `AllContent` |
/// | all content, optional view, `Stable` | `ChangedCells` / `GridWork::AllContent` |
/// | content, optional view, `Scroll`/`Rebuild` | `FullRebuild` / `GridWork::Fresh` |
/// | any geometry, any delta | `FullRebuild` / `GridWork::Fresh` |
///
/// Rules that must remain explicit (Stage 3 global constraints has the
/// rationale behind each):
///
/// - a view mark does not exclude `OverlayOnly` — `Scroll` is attempted first,
///   and a stable in-viewport selection move falls back to `OverlayOnly`;
/// - a legacy overlay-only wakeup (no `view` mark at all) may still select
///   `ScrollBlit` when the live geometric delta is a safe scroll — this is
///   also the renderer's own correctness fallback for a host that moved the
///   view without calling `view_changed`;
/// - stable content plus view uses `DamagedRows` or `ChangedCells`; content plus a
///   real scroll or rebuild plans `FullRebuild`, never a blit over changed values;
/// - `ContentWork::Rows` carries its original sheet into `GridWork::Rows`;
/// - Rows fall back to `AllContent` whenever `DamagedRows` is ineligible;
/// - geometry work forces `FullRebuild` even when `delta` is otherwise `Stable`.
///
/// `OverlayWork` is calculated once here, from the captured selection
/// visibility and the attempted work, so every execution arm reads
/// `plan.overlay` instead of re-deriving `must_paint_overlay`:
///
/// - `OverlayOnly` and `ScrollBlit` always paint it (unconditionally, in their own
///   arms — this function only needs to compute the conditional cases);
/// - `FullRebuild` always paints it — candidate geometry or model identity may
///   have changed, so a stale overlay could show handles or a selection
///   rect positioned against pixels that no longer match;
/// - `DamagedRows`/`ChangedCells` content work paints it when overlay
///   work is marked, or when captured selection visibility is true (content
///   then implies an active-cell repaint); otherwise they preserve it —
///   selection painting is disabled, so there is no active-cell repaint to
///   surface.
pub(crate) fn plan_frame(
    work: PendingWork,
    delta: FrameDelta,
    sheet: u32,
    show_selection: bool,
) -> FramePlan {
    let rebuild_reason = match delta {
        FrameDelta::Rebuild(reason) => Some(reason),
        _ => None,
    };
    // `FrameDelta::Stable` is only ever produced past `Chrome::classify`'s
    // `prev = None` guard, so it already implies a committed frame exists —
    // no separate `last_frame.is_some()` check is needed here.
    let reusable = matches!(delta, FrameDelta::Stable);

    // Computed once, from the captured selection visibility and the
    // attempted work, so `DamagedRows`/`ChangedCells` below never re-derive it.
    let content_overlay = if work.has_overlay() || show_selection {
        OverlayWork::Paint
    } else {
        OverlayWork::Preserve
    };

    // Geometric viewport probe, attempted before Overlay. Content and
    // geometry both bar it: content, because blitting stale pixels over
    // changed values is the recalc bug; geometry, because every current
    // geometry producer already forces a `Chrome::classify` hard break, so
    // this guard is a defensive belt for a future geometry producer that
    // doesn't happen to trip one. A view mark is NOT required — an
    // overlay-only wakeup still probes (legacy overlay-only-scroll
    // discovery), because this is also the renderer's own correctness
    // fallback for a host that moved the view without calling
    // `view_changed`.
    if !work.has_content()
        && !work.has_geometry()
        && let FrameDelta::Scroll(plan) = delta
    {
        return FramePlan {
            grid: GridWork::Blit(plan),
            overlay: OverlayWork::Paint,
            consumes: work,
            rebuild_reason,
        };
    }

    // Overlay: cheapest arm, reuses the committed frame and repaints only
    // the overlay layer. Deliberately ignores `view` — the probe above
    // already claimed every attempt whose pixels actually move, so a view
    // mark surviving to here means the movement stayed inside the
    // committed frame (ordinary arrow-key selection, the single most common
    // interaction in the app). Only content and geometry exclude this
    // fallback.
    if (work.has_overlay() || work.has_view())
        && !work.has_content()
        && !work.has_geometry()
        && reusable
    {
        return FramePlan {
            grid: GridWork::None,
            overlay: OverlayWork::Paint,
            consumes: work,
            rebuild_reason,
        };
    }

    // Damage fast path: viewport reusable, every content mark named its
    // rows, and they were recorded against the sheet still on screen.
    // Geometry bars the arm — band-clipping must not paper over a
    // geometry/theme change that happens to keep SlotsReuse validity. A
    // stable view mark does not: `Chrome::classify` has already proved the
    // committed geometry did not move, so only the named bands need paint.
    if !work.has_geometry()
        && reusable
        && let ContentWork::Rows {
            sheet: rows_sheet,
            spans,
        } = work.content()
        && *rows_sheet == sheet
    {
        let rows_sheet = *rows_sheet;
        let spans = spans.clone();
        return FramePlan {
            grid: GridWork::Rows {
                sheet: rows_sheet,
                spans,
            },
            overlay: content_overlay,
            consumes: work,
            rebuild_reason,
        };
    }

    // Stable all-content work can likewise reuse the committed slots
    // even when the host also marked view/overlay work. Content-free stable
    // view work is owned by the earlier Overlay arm.
    if work.has_content() && !work.has_geometry() && reusable {
        return FramePlan {
            grid: GridWork::AllContent,
            overlay: content_overlay,
            consumes: work,
            rebuild_reason,
        };
    }

    // Fallback: geometry, content plus a real scroll, or a Rebuild delta that
    // wasn't claimed above (content on a Rebuild also lands here because a
    // rebuilt frame cannot range-match the committed grid buffers).
    // Always paints the overlay — candidate geometry or model identity may
    // have changed under it.
    FramePlan {
        grid: GridWork::Fresh,
        overlay: OverlayWork::Paint,
        consumes: work,
        rebuild_reason,
    }
}
