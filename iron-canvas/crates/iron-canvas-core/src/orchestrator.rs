//! Frame dispatch and state aggregator. Backend-agnostic; the wasm-bound
//! `IronCanvas` facade in `iron-canvas-web` owns an
//! `Orchestrator<FacadeSurface>` (`WebSurface` by default,
//! `RecordingSurface<WebSurface>` under dev-tools) and delegates every
//! setter, query, and paint call here. The model is held as
//! `Rc<dyn CanvasModel>`, so the struct carries one type parameter (the
//! `Surface`), not two.
//!
//! `paint_if_dirty` takes the single queued `PendingWork` value, classifies
//! the attempt's geometric delta via `Chrome::classify`, and turns both into
//! one closed `FramePlan` via the pure `plan_frame` function — the complete
//! `PendingWork` x `FrameDelta` table lives on that function's doc comment.
//! The plan's `GridWork` selects one of five `paint_*_regime` methods
//! (cheapness-ordered: `Overlay`, `Viewport`, `Damage`, `SlotsReuse`,
//! `Fresh`). The Fresh, SlotsReuse, and Damage arms rebuild via a
//! `Chrome::next(.., FramePath::*)` walk through the matching `LayerBase`
//! paint method; the Viewport arm goes through `Chrome::prepare_blit` /
//! `Chrome::next_blit`; the Overlay arm reuses `last_frame` directly and
//! repaints only the overlay.
//!
//! Each `paint_*_regime` method prepares (bulk bridge reads, no mutation of
//! committed state) and executes (paints into the backing target) its own
//! scope, returning every healthy pane's owned cache commit as data, then
//! reduces to one
//! `PaintOutcome` — `Committed` / `Partial` / `Held` — instead of advancing
//! `last_frame`, presenting a surface, or touching `self.pending` itself.
//! [`Orchestrator::finish_attempt`] is the single completion boundary every
//! outcome flows through: it installs the attempt-owned cache commit,
//! preserves or replaces `last_frame`, presents whichever layers actually
//! painted, merges retry work back into
//! `self.pending`, and publishes
//! `last_regime`/`last_effective`/`last_work_flags`/`last_trace`. A bridge
//! failure during a regime's own bulk fetch therefore surfaces as a clean
//! `Held` (or, for `SlotsReuse`/`Damage`, a pane/row-scoped `Partial`)
//! outcome rather than a partially-applied side effect.
//!
//! The query API (`hit_test`, `cell_rect`, `resize_handle_at`,
//! `autofill_handle`) reads `last_frame`, so hits agree with painted pixels
//! by construction — including while an attempt is held, since `last_frame`
//! never advances to a candidate whose bridge reads never confirmed clean.
//!
//! Work ownership is entirely here: every setter marks intent on
//! `self.pending`, and a paint attempt consumes it with one
//! `mem::take`. Layers hold no dirty state.

use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::CanvasModel;
use crate::chrome::{BlitPlan, Chrome, FramePath, PaneRegion, PaneRegionMask, RecycledSlots};
use crate::decoration::{DecorationId, Decorations, Layer, selection::SelectionLayer};
use crate::frame_plan::{FrameDelta, FrameInputFailure, FrameInputs, RebuildReason};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::layer::{BlitPaint, LayerBase, Surface};
use crate::painter::BlitPainter;
use crate::pending_work::{ContentWork, PendingWork, RowSpan, WorkFlags};
use crate::render_overlays::RenderOverlays;
use crate::renderer::{GridRenderer, OverlayRenderer, PreparedCacheCommit};
use crate::theme::{CanvasTheme, ThemeVariables};
use crate::types::coord::{AutofillTarget, FormulaRef, RCRange, SheetArea};
use crate::types::ui::{HitTest, ResizeTarget};

/// Data-free strategy tag. Stamped by `paint_if_dirty` from
/// `FramePlan.selected_strategy` into `Orchestrator.last_regime` so
/// out-of-engine consumers (the recording pipeline) can attribute each
/// captured frame to a strategy without seeing the plan's inner data
/// (`BlitPlan`, `PaneRegionMask`, row spans — see `GridWork`). Serializes
/// with snake_case variant names to match the `.icr` JSON-lines schema.
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

/// What `plan_frame` decided the grid needs this attempt. Each variant
/// carries exactly the payload its matching `paint_*_regime` arm needs —
/// the same shapes the former payload-bearing `PaintRegime` carried, before
/// planning and execution were split into their own closed types.
///
/// `GridWork` alone determines candidate `Chrome` construction exhaustively:
///
/// | `GridWork` | candidate geometry |
/// | --- | --- |
/// | `None` | borrow committed `Chrome` |
/// | `Fresh` | fresh `Chrome` walk |
/// | `Panes(_)` | slots-reused `Chrome` |
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
    /// Full rebuild: `FramePath::Fresh` construction, every pane repainted.
    Fresh,
    /// `FramePath::SlotsReuse` construction; only `mask`'s panes refetch and
    /// repaint.
    Panes(PaneRegionMask),
    /// `FramePath::SlotsReuse` construction; only the named row bands —
    /// on `sheet`, the sheet the content work was originally recorded
    /// against — refetch and repaint via the blit-strip machinery.
    Rows { sheet: u32, spans: Vec<RowSpan> },
    /// `Chrome::next_blit` construction; the kept band ships via
    /// `Painter::blit` and only the plan's repaint strip refetches.
    Blit(BlitPlan),
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

/// The closed output of `plan_frame`: everything `paint_if_dirty` needs to
/// dispatch one paint attempt, plus the taken `PendingWork` the plan was
/// built from — owned here so a held/retried arm has it to merge back into
/// `self.pending` without a second, separate borrow of the pre-take value.
pub(crate) struct FramePlan {
    /// Stamped into `Orchestrator.last_regime` before dispatch. May diverge
    /// from what actually painted — see `FrameTrace::effective`'s doc for
    /// the selected-Viewport/effective-Fresh case, which this field does
    /// not itself encode.
    selected_strategy: PaintRegimeTag,
    grid: GridWork,
    overlay: OverlayWork,
    /// The attempt's taken `PendingWork`, owned by the plan so a held
    /// execution arm (`paint_viewport_regime`'s whole-frame hold) can merge
    /// it back into `self.pending` verbatim.
    consumes: PendingWork,
    /// Which hard break or scroll incompatibility fired, when `grid` is
    /// `Fresh` because of one. Carried for diagnostic parity with
    /// `Chrome::classify`'s verdict; not yet surfaced through `FrameTrace`
    /// — a later stage may wire it in.
    #[allow(dead_code)]
    rebuild_reason: Option<RebuildReason>,
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
/// | overlay/view only, `Stable` | `Overlay` / `GridWork::None` |
/// | overlay/view only, `Scroll(plan)` | `Viewport` / `GridWork::Blit(plan)` |
/// | overlay/view only, `Rebuild` | `Fresh` / `GridWork::Fresh` |
/// | row content only, `Stable`, sheet matches | `Damage` / `GridWork::Rows` |
/// | row content only, `Stable`, sheet differs | `SlotsReuse` / `Panes(ALL)` |
/// | row or pane content only, `Scroll`/`Rebuild` | `Fresh` / `GridWork::Fresh` |
/// | pane content only, `Stable` | `SlotsReuse` / `GridWork::Panes(mask)` |
/// | content plus view, any delta | `Fresh` / `GridWork::Fresh` |
/// | any geometry, any delta | `Fresh` / `GridWork::Fresh` |
///
/// Rules that must remain explicit (Stage 3 global constraints has the
/// rationale behind each):
///
/// - a view mark does not exclude `Overlay` — `Scroll` is attempted first,
///   and a stable in-viewport selection move falls back to `Overlay`;
/// - a legacy overlay-only wakeup (no `view` mark at all) may still select
///   `Viewport` when the live geometric delta is a safe scroll — this is
///   also the renderer's own correctness fallback for a host that moved the
///   view without calling `view_changed`;
/// - content plus view always plans `Fresh`, never a blit over changed
///   values or a band-clipped `Damage`;
/// - `ContentWork::Rows` carries its original sheet into `GridWork::Rows`;
/// - Rows imply `PaneRegionMask::ALL` whenever a mask is needed instead —
///   row precision picks `Damage`, it never narrows the pane set, so a
///   failed Damage choice (sheet mismatch) is never narrowed to visible
///   panes;
/// - geometry work forces `Fresh` even when `delta` is otherwise `Stable`.
///
/// `OverlayWork` is calculated once here, from the captured selection
/// visibility and the attempted work, so every execution arm reads
/// `plan.overlay` instead of re-deriving `must_paint_overlay`:
///
/// - `Overlay` and `Viewport` always paint it (unconditionally, in their own
///   arms — this function only needs to compute the conditional cases);
/// - `Fresh` always paints it — candidate geometry or model identity may
///   have changed, so a stale overlay could show handles or a selection
///   rect positioned against pixels that no longer match;
/// - `Damage`/`SlotsReuse` (row or pane content work) paint it when overlay
///   work is marked, or when captured selection visibility is true (content
///   then implies an active-cell repaint); otherwise they preserve it —
///   selection painting is disabled, so there is no active-cell repaint to
///   surface.
fn plan_frame(work: PendingWork, delta: FrameDelta, sheet: u32, show_selection: bool) -> FramePlan {
    let rebuild_reason = match delta {
        FrameDelta::Rebuild(reason) => Some(reason),
        _ => None,
    };
    // `FrameDelta::Stable` is only ever produced past `Chrome::classify`'s
    // `prev = None` guard, so it already implies a committed frame exists —
    // no separate `last_frame.is_some()` check is needed here.
    let reusable = matches!(delta, FrameDelta::Stable);

    // Computed once, from the captured selection visibility and the
    // attempted work, so `Damage`/`SlotsReuse` below never re-derive it.
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
            selected_strategy: PaintRegimeTag::Viewport,
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
            selected_strategy: PaintRegimeTag::Overlay,
            grid: GridWork::None,
            overlay: OverlayWork::Paint,
            consumes: work,
            rebuild_reason,
        };
    }

    // Damage fast path: viewport reusable, every content mark named its
    // rows, and they were recorded against the sheet still on screen.
    // Geometry bars the arm — band-clipping must not paper over a
    // geometry/theme change that happens to keep SlotsReuse validity. So
    // does view: a movement reaching this far needs more than the named
    // bands re-derived (the content-plus-view row).
    if !work.has_geometry()
        && !work.has_view()
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
            selected_strategy: PaintRegimeTag::Damage,
            grid: GridWork::Rows {
                sheet: rows_sheet,
                spans,
            },
            overlay: content_overlay,
            consumes: work,
            rebuild_reason,
        };
    }

    if !work.has_geometry() && !work.has_view() && reusable {
        let mask = match work.content() {
            ContentWork::Panes(mask) => *mask,
            // Rows imply the whole grid whenever a mask is needed: row
            // precision picks `Damage`, it never narrows the pane set.
            // Reaching here means `Damage` was ineligible (sheet
            // mismatch), so the fallback must stay whole-grid rather than
            // intersect the spans with what happens to be visible.
            ContentWork::Rows { .. } => PaneRegionMask::ALL,
            // Overlay-only work on a reusable frame is claimed above;
            // anything landing here without content is a conservative
            // whole-grid refresh.
            ContentWork::Clean => PaneRegionMask::ALL,
        };
        return FramePlan {
            selected_strategy: PaintRegimeTag::SlotsReuse,
            grid: GridWork::Panes(mask),
            overlay: content_overlay,
            consumes: work,
            rebuild_reason,
        };
    }

    // Fallback: geometry, content plus view, or a Rebuild delta that wasn't
    // claimed above (row/pane content on a Rebuild also lands here — a
    // rebuilt frame's pane buffers can't be range-matched against it).
    // Always paints the overlay — candidate geometry or model identity may
    // have changed under it.
    FramePlan {
        selected_strategy: PaintRegimeTag::Fresh,
        grid: GridWork::Fresh,
        overlay: OverlayWork::Paint,
        consumes: work,
        rebuild_reason,
    }
}

/// What one `paint_if_dirty` call did. `Retry` means work was retained
/// (whole-frame hold or pane-local partial commit — the trace's per-pane
/// verdicts tell which) and the scheduler must keep the loop armed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintResult {
    Idle,
    Painted,
    Retry,
}

/// What one pane's paint call decided this frame. Mirrors `RepaintPlan`
/// plus the two outcomes the planner never produces, so every exit from
/// `render_pane` / `render_pane_damage` / `execute_blit` maps to exactly
/// one variant — the relationship `PaintRegimeTag` already has to
/// `FramePlan`.
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
/// preflight aborts *before* the caller shifts a single pixel:
/// `RendererCore::prepare_blit` returns `None` on the first pane's bridge
/// failure, and `paint_grid_blit` returns without ever calling
/// `Painter::blit`. Recording that as one pane's verdict would imply the
/// other panes painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameOutcome {
    #[default]
    Painted,
    /// A `SlotsReuse`/`Damage` attempt in which some, but not all, of the
    /// target panes' fetches failed: the named panes held their prior
    /// pixels/buffers while every other targeted pane committed and
    /// presented. Distinct from `HeldOnBridgeFailure`, which names a
    /// whole-frame hold where nothing committed at all.
    PartialCommit(PaneRegionMask),
    HeldOnBridgeFailure(PaneRegion),
    /// `FrameInputs::capture` failed before dispatch reached a regime at
    /// all — no candidate geometry, no cache invalidation, no paint. See
    /// `paint_if_dirty`'s capture-failure handling.
    HeldOnInputFailure(FrameInputFailure),
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
    /// The regime that actually painted pixels this frame. Equal to `regime`
    /// except when a `Viewport` blit rejected in-place reuse and fell
    /// through to a full repaint (`BlitOutcome::FreshFallback`) —
    /// `plan_frame`'s selection and the executor's actual work diverge, and
    /// this field names the latter. `None` before the first paint, alongside
    /// `regime`.
    pub effective: Option<PaintRegimeTag>,
    /// Diagnostic projection of the `PendingWork` snapshot `plan_frame`
    /// acted on. Included because the regime alone cannot explain itself:
    /// `SlotsReuse` is the fallthrough arm, so seeing it tells you which
    /// arms were *rejected* only once you know which categories carried
    /// work.
    pub work: WorkFlags,
    /// Indexed by `PaneRegion as usize`. `None` = pane not visited this frame.
    pub panes: [Option<PaneVerdict>; 4],
    pub outcome: FrameOutcome,
    /// Set when a `Viewport` frame had to abandon the strip path for a pane.
    /// Still the expensive case even though `prepare_full_pane` needs only
    /// one bridge crossing: the pane still pays a whole-pane five-pass walk
    /// on a frame that was supposed to repaint a strip.
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
        write!(f, "[{:?}]", self.work)?;
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
        write!(f, " fetched={}", self.fetched_cell_slots)?;
        // Only printed on divergence (a `FreshFallback`) so the ordinary
        // line stays exactly as short as before this field existed.
        if self.effective != self.regime {
            match self.effective {
                Some(e) => write!(f, " eff:{e:?}")?,
                None => f.write_str(" eff:-")?,
            }
        }
        Ok(())
    }
}

/// Which surfaces `finish_attempt` must flush for a `Committed`/`Partial`
/// outcome. Grid presentation is tracked explicitly per attempt (an
/// `Overlay` regime never painted the grid at all; a `SlotsReuse`/`Damage`
/// attempt with every pane fingerprint-skipped still needs it, since the
/// prior frame's pixels are already correct on screen and nothing new was
/// drawn — see each regime helper's own construction site). Overlay
/// presentation is not tracked here: it is driven directly by this
/// attempt's `OverlayWork` verdict inside `finish_attempt`, since paint and
/// present are always paired 1:1 for the overlay layer.
#[derive(Clone, Copy, Default)]
struct PaintedLayers {
    grid: bool,
}

/// What `finish_attempt` does to `Orchestrator::last_frame` for one
/// outcome. `Preserve` covers two distinct cases that both mean "do not
/// touch the field": an `Overlay` attempt never had a candidate to begin
/// with, and an atomically-held `Fresh` attempt deliberately never took
/// `last_frame` out of `self` during preparation (see `paint_fresh_regime`)
/// so there is nothing to put back.
// `Chrome` is large and intentionally carried by value here, matching
// `chrome::blit`'s own `#[allow(clippy::result_large_err)]` precedent on
// `try_blit_reuse`/`Chrome::prepare_blit`/`Chrome::next_blit`: boxing it
// would add a heap allocation to every committed paint attempt, not just
// the rare held/rollback path clippy's size comparison is really about.
#[allow(clippy::large_enum_variant)]
enum FrameUpdate {
    Preserve,
    Replace(Chrome),
}

/// This attempt's already-captured `FrameInputs` sample plus the model and
/// `plan_frame`'s `OverlayWork` verdict — everything `finish_attempt` needs
/// to refresh and (conditionally) repaint the overlay against the frame
/// that will be committed. `None` only for the capture-failure attempt,
/// which never reaches a regime and so never refreshes overlay state at
/// all. Bundled rather than three loose parameters so it is impossible to
/// pass `inputs` without the `model`/`overlay_work` it was captured
/// alongside.
struct OverlayContext<'a> {
    model: &'a dyn CanvasModel,
    inputs: &'a FrameInputs,
    work: OverlayWork,
}

/// Private completion outcome for one paint attempt — the one value every
/// `paint_*_regime` preparation/execution helper reduces to, and the only
/// thing `finish_attempt` accepts. See the Stage 4 design doc (transactional
/// render pipeline) for the full contract this type closes over; in brief:
///
/// - `Committed`: every touched pane executed. `frame` may still be
///   `FrameUpdate::Preserve` (the `Overlay` regime never had a grid
///   candidate).
/// - `Partial`: `SlotsReuse`/`Damage` only — some, but not all, target
///   panes executed. Always carries a real replacement `Chrome`: even the
///   held panes' geometry is unchanged (a `Stable`-delta precondition), so
///   there is always a safe candidate to install, just never nothing to
///   install.
/// - `Held`: nothing executed and nothing may be presented, cached, or
///   observed as a geometry change. `frame` still needs a `FrameUpdate`
///   (not always `Preserve`) because a regime that *did* take ownership of
///   `last_frame` to build its candidate (`Viewport`'s blit, or a
///   `Viewport`-selected `FreshFallback`) must hand back an equivalent
///   value — the alternative would leave `last_frame` stuck at `None` for
///   the rest of the attempt's synchronous call chain. This is the one
///   deliberate deviation from the plan's literal `Held { retry, trace }`
///   shape: the invariant it protects (one function performs the actual
///   `self.last_frame = ..` assignment) still holds, because every
///   `FrameUpdate` a regime constructs here is either `Preserve` (nothing
///   was ever taken) or an already-resolved, zero-clone value the regime
///   had to build anyway to decide Held in the first place.
enum PaintOutcome {
    Committed {
        painted_layers: PaintedLayers,
        cache_commit: PreparedCacheCommit,
        frame: FrameUpdate,
        effective: PaintRegimeTag,
        outcome: FrameOutcome,
    },
    Partial {
        painted_layers: PaintedLayers,
        cache_commit: PreparedCacheCommit,
        frame: Chrome,
        retry: PendingWork,
        effective: PaintRegimeTag,
        outcome: FrameOutcome,
    },
    Held {
        retry: PendingWork,
        frame: FrameUpdate,
        outcome: FrameOutcome,
    },
}

/// Build the retry value for a pane-local partial commit (`SlotsReuse`,
/// `Fresh`): only the held panes' content comes back. Overlay work is
/// deliberately not included — the overlay already painted and presented
/// on this frame, so re-marking it would repaint identical pixels every
/// tick until the bridge recovers. Merged into `self.pending` exactly once,
/// by `finish_attempt` — never assigned, so a producer that queued new work
/// while this paint ran is not displaced.
fn retry_for_held_panes(held: PaneRegionMask) -> PendingWork {
    let mut retry = PendingWork::default();
    retry.mark_panes(held);
    retry
}

/// Build the retry value for a held `Damage` strip: the original sheet and
/// row spans, so the next attempt keeps the band clipping instead of
/// escalating to a whole-pane walk. Same merge-not-assign contract as
/// [`retry_for_held_panes`].
fn retry_for_held_rows(sheet: u32, spans: &[RowSpan]) -> PendingWork {
    let mut retry = PendingWork::default();
    for span in spans {
        retry.mark_rows(sheet, *span);
    }
    retry
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
    /// good — see `chrome::recycled_slots`'s module doc. `paint_fresh_regime`
    /// takes this pool's vectors to build, then folds the *outgoing*
    /// committed frame's vectors back in once the candidate has replaced it.
    spare_slots: RecycledSlots,
    /// Logical (CSS) canvas size; written by `resize`, read when building
    /// the next `Chrome`.
    size: CanvasSize,
    /// DPR from the last `resize` call. `None` before the first resize —
    /// not a `0.0` sentinel, since `resize` must self-invalidate on the
    /// very first call regardless of what DPR it's given. Private and
    /// unexposed — distinct from the wasm facade's own `last_dpr` in
    /// `iron-canvas-web`, which keeps an independent copy for the
    /// recording/playback pipeline.
    last_dpr: Option<f64>,
    /// Everything queued for the next paint attempt: geometry rebuild, view
    /// movement, content damage, overlay repaint. The single owner of paint
    /// work — layers hold none. Every setter marks intent here; a paint
    /// attempt consumes it with one `mem::take`, so successful consumption
    /// needs no end-of-paint clearing assignment. Only a regime's own retry
    /// rule merges work back in.
    pending: PendingWork,
    /// Last regime `paint_if_dirty` dispatched. Stamped from
    /// `FramePlan.selected_strategy` after `plan_frame`, read by the
    /// recording pipeline via `last_regime()`. `None` before
    /// the first paint. Plain field — `paint_if_dirty` already holds
    /// `&mut self`, so no interior mutability is needed.
    last_regime: Option<PaintRegimeTag>,
    /// The regime that actually ran, once dispatch may have overridden its
    /// own selection (see `FrameTrace::effective`). Set to `last_regime`'s
    /// value at dispatch; `paint_viewport_regime`'s `FreshFallback` arm is
    /// the only site that overwrites it afterward.
    last_effective: Option<PaintRegimeTag>,
    /// Diagnostic projection of the work the last `paint_if_dirty` took.
    /// Empty before the first paint.
    last_work_flags: WorkFlags,
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
            model_generation: 0,
            last_frame: None,
            spare_slots: RecycledSlots::default(),
            size: CanvasSize { w: 0.0, h: 0.0 },
            last_dpr: None,
            pending: PendingWork::default(),
            last_regime: None,
            last_effective: None,
            last_work_flags: WorkFlags::empty(),
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

    /// Diagnostic projection of the work the last `paint_if_dirty` acted
    /// upon. Empty before the first paint.
    pub fn last_work_flags(&self) -> WorkFlags {
        self.last_work_flags
    }

    /// Resize both layers in one call. No public per-layer resize, so
    /// callers can't leave the pair half-sized. Self-invalidating: a real
    /// size or DPR change forces the next `paint_if_dirty` to `Fresh` — no
    /// caller needs a follow-up `request_repaint()`.
    pub fn resize(&mut self, size: CanvasSize, dpr: f64) {
        if size == self.size && self.last_dpr == Some(dpr) {
            return;
        }
        self.size = size;
        self.last_dpr = Some(dpr);
        self.grid.resize(size, dpr);
        self.overlay.resize(size, dpr);
        // A backing-store resize may clear both canvases (Canvas2D), so
        // geometry invalidation must be atomic with the resize itself.
        self.last_frame = None;
        self.pending.mark_geometry();
        self.pending.mark_overlay();
    }

    /// Conservative repaint blanket. Marks geometry so the next
    /// `paint_if_dirty` falls to `Fresh` — the cheaper `SlotsReuse` /
    /// `Viewport` arms gate on geometry being clean. Adds geometry plus
    /// overlay work; it never *adds* content work, which is reserved for
    /// real cell-value changes via `mark_content_dirty`.
    ///
    /// Content and view work already queued is preserved rather than
    /// cleared. Dropping it here would strand an edit that arrived earlier
    /// in the same tick: the escalated `Fresh` frame would rebuild geometry
    /// but skip the pane-cache invalidation that only content work
    /// triggers, and repaint the stale cached values.
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
    /// panes prepare and before its first draw, an eager call here is a
    /// second, redundant painter state transition. Leaving it out also stops
    /// a *held* theme Fresh from touching the painter at all. Cell repaint
    /// coverage does not depend on it either way: `invalidate_paint_cache`
    /// only resets painter ctx state, and a Fresh candidate forces
    /// `RepaintPlan::Full` without consulting the content-keyed fingerprint
    /// tree (`build_prepared_full_pane`).
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
        // geometry + all-panes + overlay work marked below already forces
        // the next paint to `Fresh` regardless of `Chrome::classify`'s
        // verdict, so retaining the old committed frame only keeps query
        // geometry (`hit_test`, `cell_rect`, ...) coherent with the old
        // pixels for the window between this call and that Fresh paint —
        // including if the new model's scalar capture temporarily fails.
        // The one setter that *discards* queued work instead of adding to
        // it: rows and pane masks recorded against the outgoing model name
        // nothing in the incoming one. Replaced wholesale by the
        // worst-case value, which subsumes anything the old work could
        // have asked for.
        self.pending = PendingWork::default();
        self.pending.mark_geometry();
        self.pending.mark_panes(PaneRegionMask::ALL);
        self.pending.mark_overlay();
    }

    /// Mark the overlay dirty. Selection, autofill, formula-ref, and
    /// clipboard signals funnel through here; grid escalation on scroll /
    /// freeze / sheet / size change is owned by `paint_if_dirty` via
    /// `Chrome::classify`, not duplicated at the callsite.
    pub fn request_overlay_repaint(&mut self) {
        self.pending.mark_overlay();
    }

    /// Typed cell-content-changed signal. Marks the named panes' cached
    /// buffers stale so the next `paint_if_dirty` refetches their values
    /// from the model via the `SlotsReuse` arm (mask = these panes) —
    /// fixes the recalc bug where a formula dependent on an edited
    /// cell silently kept painting the stale cached value.
    pub fn mark_content_dirty(&mut self, mask: PaneRegionMask) {
        self.pending.mark_panes(mask);
    }

    /// Row-scoped `mark_content_dirty`: also names the damaged rows so
    /// `plan_frame` can clip the repaint to full-width bands. All escalation
    /// (cross-sheet rows, span-count cap, meeting unscoped pane work)
    /// belongs to `ContentWork`'s merge table, not to this callsite.
    ///
    /// No pane mask is recorded alongside the rows: row precision chooses
    /// the `Damage` strategy, it does not narrow the affected pane set, and
    /// every consumer that needs a mask from `Rows` reads `ALL`.
    pub fn mark_rows_damaged(&mut self, sheet: u32, span: RowSpan) {
        self.pending.mark_rows(sheet, span);
    }

    /// The view moved: scroll, selection, active cell, or sheet. Marks view
    /// plus overlay atomically — a view change always repositions overlay
    /// primitives, and splitting the two would let a caller queue movement
    /// that never repaints the selection rectangle.
    ///
    /// Intent only. Whether the movement shifts pixels (`Viewport`), stays
    /// inside the painted frame (`Overlay`), or needs a rebuild (`Fresh`) is
    /// `plan_frame`'s geometric verdict, not the caller's.
    pub fn view_changed(&mut self) {
        self.pending.mark_view();
        self.pending.mark_overlay();
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
            |id| crate::geometry::slot::row_height(model, view.sheet, id),
        );
        let left = origin_showing(
            column,
            view.left_column,
            frame.pane_set.cols.frozen_count(),
            pane.width,
            |id| crate::geometry::slot::col_width(model, view.sheet, id),
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

    /// Paint whichever layers are dirty. Classifies the attempt's geometric
    /// delta via `Chrome::classify`, plans it via `plan_frame` into one
    /// closed `FramePlan`, then dispatches on `plan.grid` into one of five
    /// named regimes: `Overlay`, `Viewport`, `Damage`, `SlotsReuse`,
    /// `Fresh`. The `match` is exhaustive — adding a `GridWork` variant
    /// breaks the build here by design.
    pub fn paint_if_dirty(&mut self) -> PaintResult {
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
        // Lift the model out so the paint methods can take `&mut self`
        // without overlapping the model borrow. The `is_none` guard above
        // makes the `else` unreachable, but `let-else` keeps it panic-free.
        let Some(model) = self.model.take() else {
            return PaintResult::Idle;
        };

        let model_dyn: &dyn CanvasModel = model.as_ref();

        // Capture-failure and retry contract (Stage 3 global constraints).
        // This runs after the model/pending early exits above but before
        // delta classification, plan construction, Chrome mutation, cache
        // invalidation, paint, or presentation — a failure here can hold
        // the whole attempt having touched none of those. DPR defaults to
        // `1.0` before the first `resize`, matching the renderer's own
        // default transform.
        let dpr = self.last_dpr.unwrap_or(1.0);
        let capture = FrameInputs::capture(
            model_dyn,
            self.size,
            dpr,
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
                // Capture failure never reaches a regime at all — no
                // candidate geometry, no cache invalidation, no paint — so
                // it routes through `finish_attempt` as a `Held` outcome
                // with nothing to install: `frame: FrameUpdate::Preserve`
                // (nothing was ever taken) and the complete taken `work` as
                // its retry.
                let result = self.finish_attempt(
                    None,
                    flags,
                    None,
                    PaintOutcome::Held {
                        retry: work,
                        frame: FrameUpdate::Preserve,
                        outcome: FrameOutcome::HeldOnInputFailure(failure),
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
        let plan = plan_frame(work, delta, inputs.sheet(), inputs.show_selection());
        let selected = plan.selected_strategy;
        let work_flags = plan.consumes.flags();
        // Clear before dispatch so the trace `finish_attempt` reads back
        // describes this frame only. An `Overlay` regime legitimately
        // leaves every pane `None` — it never calls a grid pane renderer.
        self.grid.renderer.reset_trace();
        // `plan.consumes` is the attempt's taken `PendingWork`, owned by the
        // plan; moving it out here (rather than a second borrow of the
        // pre-take value) is what lets a held arm's `PaintOutcome` carry it
        // straight back to `finish_attempt`'s merge step. An arm that fully
        // commits does nothing further with it; only an arm that holds (in
        // full or in part) constructs its own retry scope from it.
        let overlay_work = plan.overlay;
        let work = plan.consumes;
        let outcome = match plan.grid {
            GridWork::None => self.paint_overlay_regime(),
            GridWork::Blit(blit_plan) => {
                self.paint_viewport_regime(model_dyn, &inputs, blit_plan, work)
            }
            GridWork::Panes(mask) => self.paint_slots_reuse_regime(model_dyn, &inputs, mask, work),
            GridWork::Fresh => self.paint_fresh_regime(model_dyn, &inputs, work),
            GridWork::Rows { sheet, spans } => {
                self.paint_damage_regime(model_dyn, &inputs, sheet, spans, work)
            }
        };
        // Every dispatched regime's own precondition (a `Stable`/`Scroll`
        // delta, or `plan_frame`'s `reusable` gate) already proves
        // `last_frame.is_some()` before it takes it — `None` here means
        // that invariant broke, and the defensive fallback is to treat the
        // attempt as if nothing had been raised at all, exactly like the
        // pre-capture empty-work short circuit above.
        let Some(outcome) = outcome else {
            self.model = Some(model);
            return PaintResult::Idle;
        };

        let overlay_ctx = Some(OverlayContext {
            model: model_dyn,
            inputs: &inputs,
            work: overlay_work,
        });
        let result = self.finish_attempt(Some(selected), work_flags, overlay_ctx, outcome);

        // Restore site for every regime that reached dispatch. The other
        // restore site is the capture-failure early return above, which
        // returns before any regime runs.
        self.model = Some(model);
        result
    }

    /// The one function that completes a paint attempt. Every
    /// `paint_*_regime` preparation/execution helper reduces to a
    /// `PaintOutcome`; this is the completion boundary that advances or
    /// preserves `last_frame`, refreshes and (conditionally) repaints the
    /// overlay against the frame that will be committed, presents whichever
    /// surfaces actually painted, merges retry work back into
    /// `self.pending`, and publishes
    /// `last_regime`/`last_effective`/`last_work_flags`/`last_trace`. It also
    /// installs the aggregate cache commit before publishing/presenting the
    /// matching frame.
    ///
    /// `selected` and `work_flags` come from `plan_frame`'s verdict —
    /// known before dispatch, so every outcome (including a `Held` capture
    /// failure, which never reaches a regime) can still stamp them.
    /// `overlay_ctx` is `None` only for that capture-failure case: a
    /// `Held` outcome never refreshes overlay state regardless, so the
    /// context simply isn't there to consult on that branch.
    fn finish_attempt(
        &mut self,
        selected: Option<PaintRegimeTag>,
        work_flags: WorkFlags,
        overlay_ctx: Option<OverlayContext<'_>>,
        outcome: PaintOutcome,
    ) -> PaintResult {
        let (painted_layers, cache_commit, frame, retry, effective, frame_outcome, result) =
            match outcome {
                PaintOutcome::Committed {
                    painted_layers,
                    cache_commit,
                    frame,
                    effective,
                    outcome,
                } => (
                    Some(painted_layers),
                    Some(cache_commit),
                    frame,
                    None,
                    Some(effective),
                    outcome,
                    PaintResult::Painted,
                ),
                PaintOutcome::Partial {
                    painted_layers,
                    cache_commit,
                    frame,
                    retry,
                    effective,
                    outcome,
                } => (
                    Some(painted_layers),
                    Some(cache_commit),
                    FrameUpdate::Replace(frame),
                    Some(retry),
                    Some(effective),
                    outcome,
                    PaintResult::Retry,
                ),
                PaintOutcome::Held {
                    retry,
                    frame,
                    outcome,
                } => (
                    None,
                    None,
                    frame,
                    Some(retry),
                    None,
                    outcome,
                    PaintResult::Retry,
                ),
            };

        // 1. install the attempt-owned cache commit, then publish the frame
        //    whose pixels and cache metadata it describes. Held outcomes
        //    carry no commit and therefore touch neither persistent cache nor
        //    frame state beyond their explicit rollback/preserve update.
        if let Some(cache_commit) = cache_commit {
            self.grid.commit_pane_cache(cache_commit);
        }
        self.install_frame(frame);

        if let Some(layers) = painted_layers {
            if let Some(ctx) = overlay_ctx {
                // Committed and Partial refresh committed selection/
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
                    // before; every other regime reads the frame
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
            // 3. present the grid iff the outcome says grid pixels executed.
            if layers.grid {
                self.grid.present();
            }
        }
        // Held refreshes nothing and paints/presents no overlay — the
        // branch above is simply never entered.

        // 4. merge retry work into any work raised during the attempt.
        if let Some(retry) = retry {
            self.pending.merge(retry);
        }

        // 5. publish last_regime, last_effective, last_work_flags, and
        //    last_trace — built once here from plan metadata (`selected`/
        //    `work_flags`), the renderer's own prepared-fetch attribution
        //    and pane verdicts (`self.grid.renderer.trace()`), and this
        //    outcome's effective strategy/`FrameOutcome`.
        self.last_regime = selected;
        self.last_effective = effective;
        self.last_work_flags = work_flags;
        let mut trace = self.grid.renderer.trace();
        trace.regime = selected;
        trace.effective = effective;
        trace.work = work_flags;
        trace.outcome = frame_outcome;
        self.last_trace = trace;

        // 6. return PaintResult::Painted or PaintResult::Retry.
        result
    }

    /// Preserve or replace `last_frame`, recycling the outgoing frame's
    /// slot Vecs into `spare_slots` whenever one is actually displaced —
    /// the only consumer of that pool is the next `Fresh` attempt's
    /// `Chrome::build` (see `chrome::recycled_slots`'s module doc). A
    /// `Preserve` update never touches either field: the regime that
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

    /// Overlay-only fast path. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. `plan_frame` proves the
    /// preconditions (slot vecs still match, `last_frame` is `Some`).
    /// Overlay-only fast path: reuses the committed frame verbatim, no grid
    /// touch at all. Preparation/execution helper only — `finish_attempt`
    /// does the actual overlay refresh/paint/present, using this outcome's
    /// `FrameUpdate::Preserve` to read `self.last_frame` as it already
    /// stands. `None` is the defensive fallback for `plan_frame`'s own
    /// precondition (a `Stable` delta already implies a committed frame);
    /// `paint_if_dirty` treats it as a plain Idle, touching no state.
    fn paint_overlay_regime(&self) -> Option<PaintOutcome> {
        self.last_frame.as_ref()?;
        Some(PaintOutcome::Committed {
            painted_layers: PaintedLayers { grid: false },
            cache_commit: PreparedCacheCommit::default(),
            frame: FrameUpdate::Preserve,
            effective: PaintRegimeTag::Overlay,
            outcome: FrameOutcome::Painted,
        })
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
    /// `Err(prev)` arm is the demote-to-`Fresh` path (e.g. a row-header
    /// digit boundary rejects in-place reuse), delegated to
    /// [`Self::paint_fresh_fallback`] — the same atomic-Fresh mechanics
    /// `paint_fresh_regime` uses, since a `FreshFallback`'s geometry and
    /// full-canvas background differ from the committed frame exactly like
    /// an ordinary Fresh rebuild's do.
    fn paint_viewport_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: BlitPlan,
        work: PendingWork,
    ) -> Option<PaintOutcome> {
        let prev = self.last_frame.take()?;
        match Chrome::prepare_blit(prev, model, inputs, &plan) {
            Ok(prepared) => {
                let cache_commit = match self.grid.paint_grid_blit(model, prepared.frame(), &plan) {
                    BlitPaint::Painted(cache_commit) => cache_commit,
                    BlitPaint::Held => {
                        // Whole-frame hold: the preflight aborted before a pixel
                        // shifted, so nothing at all was committed.
                        // `prepare_blit`'s own `render_grid_blit` already
                        // stamped the renderer trace's `FrameOutcome` via
                        // `trace_frame_held` — read it back rather than
                        // re-deriving which pane triggered the hold.
                        let outcome = self.grid.renderer.trace().outcome;
                        return Some(PaintOutcome::Held {
                            retry: work,
                            // `rollback` moves `prev`'s untouched pieces back
                            // out of the now-discarded candidate — no clone was
                            // ever taken — so this is exactly what `last_frame`
                            // held before the attempt; the entire attempt
                            // (including the overlay mark, which never painted)
                            // comes back via `retry`.
                            frame: FrameUpdate::Replace(prepared.rollback()),
                            outcome,
                        });
                    }
                };
                Some(PaintOutcome::Committed {
                    painted_layers: PaintedLayers { grid: true },
                    cache_commit,
                    frame: FrameUpdate::Replace(prepared.commit()),
                    effective: PaintRegimeTag::Viewport,
                    outcome: FrameOutcome::Painted,
                })
            }
            Err(prev) => self.paint_fresh_fallback(model, inputs, work, prev),
        }
    }

    /// Shared Fresh-construction tail for `paint_fresh_regime` and
    /// `paint_viewport_regime`'s `FreshFallback` sub-path: builds a
    /// `Fresh`-kind candidate from `self.spare_slots` (never touching
    /// `self.last_frame` — see the module's Stage 4 design doc's Fresh
    /// recipe) and paints every pane atomically via
    /// `RendererCore::render_panes_atomic`. Returns the candidate and its
    /// held-mask verdict, which is always exactly `PaneRegionMask::ALL`
    /// (nothing committed) or `PaneRegionMask::EMPTY` (everything did) —
    /// never a partial value. The caller still owns the held-vs-committed
    /// `FrameUpdate`/pool-recycling decision, because the two callers
    /// differ in what (if anything) they must hand back on Held: ordinary
    /// Fresh never took `last_frame` at all, but `FreshFallback` already
    /// took it for the original blit attempt and holds `prev` locally.
    fn build_and_paint_fresh(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
    ) -> (Chrome, Option<PreparedCacheCommit>) {
        let spare = std::mem::take(&mut self.spare_slots);
        let frame = Chrome::build(model, inputs, spare);
        // `paint_grid_fresh` prepares every pane before touching the
        // painter at all (not even the cache invalidation or background
        // fill), so a held attempt is a true no-op here — see its doc.
        let cache_commit = self
            .grid
            .paint_grid_fresh(model, &frame, PaneRegionMask::ALL);
        (frame, cache_commit)
    }

    /// `paint_viewport_regime`'s `Err(prev)` arm: `prepare_blit` rejected
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
    ) -> Option<PaintOutcome> {
        let (frame, cache_commit) = self.build_and_paint_fresh(model, inputs);

        let Some(cache_commit) = cache_commit else {
            // Atomic hold: park the failed candidate's own vecs for reuse
            // and hand `prev` back explicitly — unlike an ordinary Fresh
            // hold, `prev` isn't sitting in `self.last_frame` for
            // `finish_attempt` to simply leave alone; it was already taken
            // out for the original blit attempt above.
            self.spare_slots = RecycledSlots::from_pane_set(frame.pane_set);
            let outcome = self.grid.renderer.trace().outcome;
            return Some(PaintOutcome::Held {
                retry: work,
                frame: FrameUpdate::Replace(prev),
                outcome,
            });
        };

        // `prev`'s own vecs are about to be displaced by `frame`; recycle
        // them into the pool exactly like an ordinary Fresh commit does in
        // `install_frame` — `self.last_frame` was already emptied for the
        // original blit attempt, so `finish_attempt`'s own recycle step
        // would otherwise find nothing to fold in.
        self.spare_slots = RecycledSlots::from_pane_set(prev.pane_set);
        Some(PaintOutcome::Committed {
            painted_layers: PaintedLayers { grid: true },
            cache_commit,
            frame: FrameUpdate::Replace(frame),
            effective: PaintRegimeTag::Fresh,
            outcome: FrameOutcome::Painted,
        })
    }

    /// True when the renderer's own trace shows at least one pane that
    /// actually executed (`Skip`/`Rows`/`Full`/`Strip`) this attempt,
    /// rather than either `Held` or never having been visited at all
    /// (`None` — a geometrically empty pane, or one outside this attempt's
    /// scope). This is the Held-vs-Partial boundary for `SlotsReuse` and
    /// `Damage`: comparing the returned held mask against the target mask
    /// directly would misclassify an attempt as partial whenever the
    /// target also names a geometrically empty pane (e.g. `TopLeft` on an
    /// unfrozen sheet), since an empty pane is never counted in the held
    /// mask at all.
    fn any_pane_painted(&self) -> bool {
        self.grid
            .renderer
            .trace()
            .panes
            .iter()
            .any(|v| matches!(v, Some(pv) if *pv != PaneVerdict::Held))
    }

    /// Damage regime: slot vecs survive (same preconditions as SlotsReuse),
    /// prior grid pixels stay, only the damaged bands refetch + repaint.
    /// No cache invalidation ahead of the paint — the strip path
    /// (`prepare_damage_pane` / `execute_damage_pane`) splices fetched bands
    /// into the pane buffers and leaves the pane fingerprint tree untouched,
    /// atomically: a transient bridge failure on any of the four strip
    /// buffers leaves that pane's buffers, pixels, range, and tree
    /// untouched instead of partially splicing.
    fn paint_damage_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        sheet: u32,
        spans: Vec<RowSpan>,
        work: PendingWork,
    ) -> Option<PaintOutcome> {
        let prev = self.last_frame.take()?;
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);
        let grid_paint = self.grid.paint_grid_damage(model, &frame, &spans);
        let held = grid_paint.held;

        if held.is_empty() {
            return Some(PaintOutcome::Committed {
                painted_layers: PaintedLayers { grid: true },
                cache_commit: grid_paint.cache_commit,
                frame: FrameUpdate::Replace(frame),
                effective: PaintRegimeTag::Damage,
                outcome: FrameOutcome::Painted,
            });
        }

        if self.any_pane_painted() {
            // Pane-local partial commit: healthy bands painted and
            // present; held bands keep their prior pixels. Retries the
            // original sheet + row spans (never a pane mask) — row
            // precision never narrows to a pane scope — reconstructed via
            // `retry_for_held_rows` rather than `work` itself, since a
            // Preserve row precision on retry. `ContentWork::Rows` has no
            // pane mask, so every intersecting pane is revisited; healthy
            // panes remain safe because each retry is freshly prepared and
            // committed through the same completion boundary.
            return Some(PaintOutcome::Partial {
                painted_layers: PaintedLayers { grid: true },
                cache_commit: grid_paint.cache_commit,
                frame,
                retry: retry_for_held_rows(sheet, &spans),
                effective: PaintRegimeTag::Damage,
                outcome: FrameOutcome::PartialCommit(held),
            });
        }
        // Every intersected pane failed: a true hold, not a zero-pixel
        // partial commit. `frame`'s geometry is content-identical to
        // `prev` — the `Stable` delta that selected this regime already
        // proves it — so installing it is exactly as safe as restoring
        // `prev` would be, with no second construction path needed.
        //
        // Retries the complete consumed `work`, not a `sheet`/`spans`
        // reconstruction: `plan_frame` derives `sheet`/`spans` FROM
        // `work.content()` in the first place, so `work` already carries
        // the identical row scope — but also whatever `view`/`overlay`
        // bits rode along with it. A held attempt never runs the overlay
        // refresh (see `finish_attempt`), so a `mark_rows_damaged` +
        // `request_overlay_repaint` pair raised together in one tick, on
        // an attempt that then fully fails, must have its overlay mark
        // survive into the retry — `retry_for_held_rows` alone would
        // silently drop it.
        Some(PaintOutcome::Held {
            retry: work,
            frame: FrameUpdate::Replace(frame),
            outcome: FrameOutcome::HeldOnBridgeFailure(
                held.regions().next().unwrap_or(PaneRegion::BottomRight),
            ),
        })
    }

    /// SlotsReuse regime: prev's slot vecs survive (viewport unchanged);
    /// `render_pane` fetches every pane in `mask` unconditionally and
    /// fingerprint-skips a pane whose refetch matches its prior committed
    /// content — no eager cache invalidation is needed ahead of the paint
    /// to force that fetch (see `PaneCache::invalidate`'s doc for the
    /// buffer-range-only distinction that made the old eager call
    /// redundant with `render_pane`'s own always-fetch, commit-on-success
    /// behavior, and actively wrong on a held pane: it would have cleared
    /// the pane's cached range before knowing the fetch would fail).
    fn paint_slots_reuse_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        mask: PaneRegionMask,
        work: PendingWork,
    ) -> Option<PaintOutcome> {
        let prev = self.last_frame.take()?;
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);
        let grid_paint = self.grid.paint_grid(model, &frame, mask);
        let held = grid_paint.held;

        if held.is_empty() {
            return Some(PaintOutcome::Committed {
                painted_layers: PaintedLayers { grid: true },
                cache_commit: grid_paint.cache_commit,
                frame: FrameUpdate::Replace(frame),
                effective: PaintRegimeTag::SlotsReuse,
                outcome: FrameOutcome::Painted,
            });
        }

        if self.any_pane_painted() {
            // Pane-local partial commit (see plan contract): painted panes
            // present; held panes keep their prior pixels. Retry narrows to
            // exactly the held scope (via `retry_for_held_panes`), not the
            // whole consumed `work` — the healthy panes must not repaint
            // again, so the retry has to be strictly narrower than what
            // was consumed.
            return Some(PaintOutcome::Partial {
                painted_layers: PaintedLayers { grid: true },
                cache_commit: grid_paint.cache_commit,
                frame,
                retry: retry_for_held_panes(held),
                effective: PaintRegimeTag::SlotsReuse,
                outcome: FrameOutcome::PartialCommit(held),
            });
        }
        // Every targeted pane failed: a true hold, not a zero-pixel partial
        // commit. `frame`'s geometry is content-identical to `prev` for
        // the same reason `paint_damage_regime` installs it on a full hold
        // too.
        //
        // Retries the complete consumed `work`, not
        // `retry_for_held_panes(mask)`: `plan_frame` derives `mask` FROM
        // `work.content()` in the first place, so `work` already carries
        // the identical pane scope (and, on the cross-sheet-rows or
        // Clean-content fallback paths, the *original* content shape
        // `mask` was normalized from) — but also whatever `view`/`overlay`
        // bits rode along with it. A held attempt never runs the overlay
        // refresh (see `finish_attempt`), so a `mark_content_dirty` +
        // `request_overlay_repaint` pair raised together in one tick, on
        // an attempt that then fully fails, must have its overlay mark
        // survive into the retry — `retry_for_held_panes(mask)` alone
        // would silently drop it.
        Some(PaintOutcome::Held {
            retry: work,
            frame: FrameUpdate::Replace(frame),
            outcome: FrameOutcome::HeldOnBridgeFailure(
                held.regions().next().unwrap_or(PaneRegion::BottomRight),
            ),
        })
    }

    /// Full grid repaint. Slot vecs walked fresh from the model; the new
    /// vecs make any cross-frame fingerprint compare meaningless, so every
    /// pane repaints. Selected when slot vecs diverged or no prior frame.
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
    fn paint_fresh_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
    ) -> Option<PaintOutcome> {
        let (frame, cache_commit) = self.build_and_paint_fresh(model, inputs);

        let Some(cache_commit) = cache_commit else {
            // Atomic hold: give the failed candidate's own vecs back to the
            // pool and leave `last_frame` completely untouched — it was
            // never taken (see `build_and_paint_fresh`), so `prev` (or
            // `None`, on a held first frame) is exactly what
            // `finish_attempt` will still see.
            self.spare_slots = RecycledSlots::from_pane_set(frame.pane_set);
            let outcome = self.grid.renderer.trace().outcome;
            return Some(PaintOutcome::Held {
                retry: work,
                frame: FrameUpdate::Preserve,
                outcome,
            });
        };

        Some(PaintOutcome::Committed {
            painted_layers: PaintedLayers { grid: true },
            cache_commit,
            frame: FrameUpdate::Replace(frame),
            effective: PaintRegimeTag::Fresh,
            outcome: FrameOutcome::Painted,
        })
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

/// `plan_frame` table coverage: every `PendingWork` category x `FrameDelta`
/// outcome cell, plus the rules the doc comment calls out by name. Named
/// `frame_plan_tests` (not `plan_tests`) so `cargo test -p iron-canvas-core
/// frame_plan` — the Task 4 brief's exact run command — collects them.
///
/// These are unit tests over the pure `plan_frame` function directly, not
/// `Orchestrator::paint_if_dirty` — `GridWork`/`OverlayWork`/`FramePlan` are
/// crate-private, so only a test module nested here (a descendant of
/// `orchestrator`, hence able to see its private items) can construct and
/// inspect them. The real-world painter-op consequence of the hot-path case
/// below is the same scenario `orchestrator_regimes.rs`'s
/// `view_only_navigation_without_a_shift_emits_no_grid_ops` drives through
/// the actual `Orchestrator` + recorder.
#[cfg(test)]
mod frame_plan_tests {
    use super::*;
    use crate::geometry::prim::Axis;

    const SHEET: u32 = 0;
    const OTHER_SHEET: u32 = 7;

    fn work_with(f: impl FnOnce(&mut PendingWork)) -> PendingWork {
        let mut work = PendingWork::default();
        f(&mut work);
        work
    }

    /// Empty `shifts` is fine: `PaneShift` isn't nameable from this module
    /// (`chrome::blit` is private to the `chrome` subtree), and the planner
    /// never inspects a `BlitPlan`'s contents — it only wraps whatever
    /// `Chrome::classify` handed it into `GridWork::Blit`.
    fn stub_scroll() -> FrameDelta {
        FrameDelta::Scroll(BlitPlan {
            axis: Axis::Row,
            shifts: Vec::new(),
            repaint_strip: PixelRect {
                top_left: Point { x: 0, y: 0 },
                width: 10,
                height: 10,
            },
        })
    }

    fn stub_rebuild() -> FrameDelta {
        FrameDelta::Rebuild(RebuildReason::Sheet)
    }

    // ── Required hot-path assertion ──

    /// `view + overlay, FrameDelta::Stable -> selected Overlay ->
    /// GridWork::None -> zero grid operations`. The single most important
    /// regression to pin: a stable, no-shift view/overlay-only attempt must
    /// plan zero grid work, or every arrow-key press regresses to a
    /// full-grid repaint.
    #[test]
    fn view_and_overlay_stable_selects_overlay_with_no_grid_work() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_overlay();
        });
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Overlay);
        assert!(
            matches!(plan.grid, GridWork::None),
            "a stable, no-shift view+overlay attempt must plan zero grid work"
        );
        assert_eq!(plan.overlay, OverlayWork::Paint);
    }

    // ── Category: overlay/view only, no content, no geometry ──

    #[test]
    fn overlay_only_stable_selects_overlay() {
        let work = work_with(|w| w.mark_overlay());
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Overlay);
        assert!(matches!(plan.grid, GridWork::None));
    }

    /// The no-shift view fallback with `view` as the *only* mark (no
    /// `overlay`) — proves the Overlay guard's `work.has_view()` disjunct
    /// specifically. Regressing this to require `has_overlay()` too would
    /// turn ordinary arrow-key navigation into a full-grid repaint.
    #[test]
    fn view_only_no_shift_still_selects_overlay() {
        let work = work_with(|w| w.mark_view());
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(
            plan.selected_strategy,
            PaintRegimeTag::Overlay,
            "view alone, with no pixel shift, must still fall back to Overlay"
        );
        assert!(matches!(plan.grid, GridWork::None));
    }

    #[test]
    fn view_and_overlay_scroll_selects_viewport() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_overlay();
        });
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Viewport);
        assert!(matches!(plan.grid, GridWork::Blit(_)));
        assert_eq!(plan.overlay, OverlayWork::Paint);
    }

    /// Legacy overlay-only scroll discovery: no `view` mark at all, only
    /// `overlay` — the probe must still claim a real geometric scroll. This
    /// is also the renderer's own correctness fallback for a host that moved
    /// the view without calling `view_changed`.
    #[test]
    fn overlay_only_scroll_still_selects_viewport() {
        let work = work_with(|w| w.mark_overlay());
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(
            plan.selected_strategy,
            PaintRegimeTag::Viewport,
            "an overlay-only wakeup must still discover a real geometric scroll"
        );
        assert!(matches!(plan.grid, GridWork::Blit(_)));
    }

    #[test]
    fn view_and_overlay_rebuild_selects_fresh() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_overlay();
        });
        let plan = plan_frame(work, stub_rebuild(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
        assert_eq!(plan.overlay, OverlayWork::Paint);
        assert_eq!(plan.rebuild_reason, Some(RebuildReason::Sheet));
    }

    // ── Category: row content only — both row-sheet outcomes ──

    #[test]
    fn row_content_stable_matching_sheet_selects_damage() {
        let work = work_with(|w| w.mark_rows(SHEET, RowSpan { r1: 2, r2: 4 }));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Damage);
        let GridWork::Rows { sheet, spans } = plan.grid else {
            panic!("expected GridWork::Rows");
        };
        assert_eq!(sheet, SHEET);
        assert_eq!(spans, vec![RowSpan { r1: 2, r2: 4 }]);
    }

    #[test]
    fn row_content_stable_mismatched_sheet_falls_back_to_slots_reuse_all() {
        let work = work_with(|w| w.mark_rows(OTHER_SHEET, RowSpan { r1: 2, r2: 4 }));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(
            plan.selected_strategy,
            PaintRegimeTag::SlotsReuse,
            "row work recorded against a sheet that isn't on screen can't clip to bands"
        );
        let GridWork::Panes(mask) = plan.grid else {
            panic!("expected GridWork::Panes");
        };
        assert_eq!(mask, PaneRegionMask::ALL);
    }

    #[test]
    fn row_content_scroll_selects_fresh() {
        let work = work_with(|w| w.mark_rows(SHEET, RowSpan { r1: 2, r2: 4 }));
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    #[test]
    fn row_content_rebuild_selects_fresh() {
        let work = work_with(|w| w.mark_rows(SHEET, RowSpan { r1: 2, r2: 4 }));
        let plan = plan_frame(work, stub_rebuild(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    // ── Category: pane content only ──

    #[test]
    fn pane_content_stable_selects_slots_reuse() {
        let work = work_with(|w| w.mark_panes(PaneRegionMask::TOP_LEFT));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::SlotsReuse);
        let GridWork::Panes(mask) = plan.grid else {
            panic!("expected GridWork::Panes");
        };
        assert_eq!(mask, PaneRegionMask::TOP_LEFT);
    }

    #[test]
    fn pane_content_scroll_selects_fresh() {
        let work = work_with(|w| w.mark_panes(PaneRegionMask::TOP_LEFT));
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    #[test]
    fn pane_content_rebuild_selects_fresh() {
        let work = work_with(|w| w.mark_panes(PaneRegionMask::TOP_LEFT));
        let plan = plan_frame(work, stub_rebuild(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    // ── Category: content plus view — always Fresh, any delta ──

    #[test]
    fn content_plus_view_stable_selects_fresh() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_rows(SHEET, RowSpan { r1: 1, r2: 1 });
        });
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(
            plan.selected_strategy,
            PaintRegimeTag::Fresh,
            "content plus view must never clip to bands or blit"
        );
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    #[test]
    fn content_plus_view_scroll_selects_fresh() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_panes(PaneRegionMask::ALL);
        });
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
    }

    #[test]
    fn content_plus_view_rebuild_selects_fresh() {
        let work = work_with(|w| {
            w.mark_view();
            w.mark_rows(SHEET, RowSpan { r1: 1, r2: 1 });
        });
        let plan = plan_frame(work, stub_rebuild(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    // ── Category: any geometry — always Fresh, any delta ──

    #[test]
    fn geometry_alone_stable_selects_fresh() {
        let work = work_with(|w| w.mark_geometry());
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    #[test]
    fn geometry_alone_rebuild_selects_fresh() {
        let work = work_with(|w| w.mark_geometry());
        let plan = plan_frame(work, stub_rebuild(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert!(matches!(plan.grid, GridWork::Fresh));
    }

    /// Mirrors `orchestrator_regimes.rs`'s
    /// `geometry_plus_real_scroll_never_dispatches_viewport`: geometry work
    /// concurrent with a real shift must never dispatch `Viewport`.
    #[test]
    fn geometry_with_everything_else_still_selects_fresh() {
        let work = work_with(|w| {
            w.mark_geometry();
            w.mark_view();
            w.mark_overlay();
            w.mark_panes(PaneRegionMask::ALL);
        });
        let plan = plan_frame(work, stub_scroll(), SHEET, true);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
    }

    // ── OverlayWork policy ──

    #[test]
    fn damage_preserves_overlay_when_selection_hidden_and_no_overlay_mark() {
        let work = work_with(|w| w.mark_rows(SHEET, RowSpan { r1: 2, r2: 2 }));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, false);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Damage);
        assert_eq!(
            plan.overlay,
            OverlayWork::Preserve,
            "content-only work must preserve the overlay when selection painting is disabled"
        );
    }

    #[test]
    fn damage_paints_overlay_when_selection_is_visible() {
        let work = work_with(|w| w.mark_rows(SHEET, RowSpan { r1: 2, r2: 2 }));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert_eq!(plan.overlay, OverlayWork::Paint);
    }

    #[test]
    fn slots_reuse_preserves_overlay_when_selection_hidden_and_no_overlay_mark() {
        let work = work_with(|w| w.mark_panes(PaneRegionMask::ALL));
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, false);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::SlotsReuse);
        assert_eq!(plan.overlay, OverlayWork::Preserve);
    }

    #[test]
    fn slots_reuse_paints_overlay_when_overlay_marked_even_with_selection_hidden() {
        let work = work_with(|w| {
            w.mark_panes(PaneRegionMask::ALL);
            w.mark_overlay();
        });
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, false);

        assert_eq!(
            plan.overlay,
            OverlayWork::Paint,
            "an explicit overlay mark must paint regardless of selection visibility"
        );
    }

    #[test]
    fn fresh_always_paints_overlay_even_with_selection_hidden() {
        let work = work_with(|w| w.mark_geometry());
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, false);

        assert_eq!(plan.selected_strategy, PaintRegimeTag::Fresh);
        assert_eq!(plan.overlay, OverlayWork::Paint);
    }

    // ── FramePlan owns the taken PendingWork ──

    #[test]
    fn plan_owns_the_taken_pending_work() {
        let work = work_with(|w| w.mark_overlay());
        let plan = plan_frame(work, FrameDelta::Stable, SHEET, true);

        assert!(plan.consumes.has_overlay());
    }
}
