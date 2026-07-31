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
//! The plan's `GridWork` selects one of five `paint_*_regime` arms
//! (cheapness-ordered: `Overlay`, `Viewport`, `Damage`, `SlotsReuse`,
//! `Fresh`). The Fresh, SlotsReuse, and Damage arms rebuild via a
//! `Chrome::next(.., FramePath::*)` walk through the matching `LayerBase`
//! paint method; the Viewport arm goes through `Chrome::next_blit`; the
//! Overlay arm reuses `last_frame` directly and repaints only the overlay.
//! The query API (`hit_test`, `cell_rect`, `resize_handle_at`,
//! `autofill_handle`) reads `last_frame`, so hits agree with painted pixels
//! by construction.
//!
//! Work ownership is entirely here: every setter marks intent on
//! `self.pending`, and a paint attempt consumes it with one
//! `mem::take`. Layers hold no dirty state.

use std::fmt;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::CanvasModel;
use crate::chrome::{BlitOutcome, BlitPlan, Chrome, FramePath, PaneRegion, PaneRegionMask};
use crate::decoration::{DecorationId, Decorations, Layer, selection::SelectionLayer};
use crate::frame_plan::{FrameDelta, FrameInputFailure, FrameInputs, RebuildReason};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Point;
use crate::layer::{BlitPaint, LayerBase, Surface};
use crate::painter::BlitPainter;
use crate::pending_work::{ContentWork, PendingWork, RowSpan, WorkFlags};
use crate::render_overlays::RenderOverlays;
use crate::renderer::{GridRenderer, OverlayRenderer};
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

/// What one pane's `render_pane*` call decided this frame. Mirrors
/// `RepaintPlan` plus the two outcomes the planner never produces, so every
/// exit from `render_pane` / `render_pane_blit` / `render_pane_strip` maps to
/// exactly one variant — the relationship `PaintRegimeTag` already has to
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
/// preflight aborts *before* any `render_pane*` runs: `prefetch_blit_strips`
/// returns `false` and `paint_grid_blit` returns without shifting a pixel.
/// Recording that as one pane's verdict would imply the other panes painted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FrameOutcome {
    #[default]
    Painted,
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
    /// invalidates the renderer paint cache and marks both layers dirty.
    /// `Chrome::classify` rejects a theme-mismatched frame itself, so the
    /// next paint reaches `Fresh` through the classifier's verdict — no
    /// out-of-band `last_frame` drop needed here. The paint-cache invalidation stays: the
    /// per-cell fingerprint cache is keyed on content, not palette, so even a
    /// Fresh rebuild would fingerprint-skip stale-color cells without it.
    pub fn set_theme(&mut self, theme: CanvasTheme) {
        if theme != *self.theme {
            self.theme = Rc::new(theme);
            self.grid.invalidate_paint_cache();
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
                // 6. stamp the attempt's `WorkFlags` before `work` moves.
                let flags = work.flags();
                // 1. merge the entire taken `PendingWork` back into
                //    `self.pending`.
                self.pending.merge(work);
                // 2. `last_frame` and decoration snapshots: untouched above.
                // 3. present neither surface — neither `present()` call below
                //    is reached.
                // 4. emit no painter operations — no paint method ran.
                // 7. selected/effective strategy is `None` for this attempt.
                self.last_regime = None;
                self.last_effective = None;
                self.last_work_flags = flags;
                // 8. stamp a typed held outcome.
                self.last_trace = FrameTrace {
                    regime: None,
                    effective: None,
                    work: flags,
                    outcome: FrameOutcome::HeldOnInputFailure(failure),
                    ..FrameTrace::default()
                };
                self.model = Some(model);
                // 5. return `PaintResult::Retry`.
                return PaintResult::Retry;
            }
        };

        let delta = Chrome::classify(
            self.last_frame.as_ref(),
            model_dyn,
            &inputs,
            self.decos.selection().active_cell.as_ref(),
        );
        let plan = plan_frame(work, delta, inputs.sheet(), inputs.show_selection());
        self.last_regime = Some(plan.selected_strategy);
        self.last_effective = self.last_regime;
        self.last_work_flags = plan.consumes.flags();
        // Clear before dispatch so the trace describes this frame only. An
        // `Overlay` regime legitimately leaves every pane `None` — it never
        // calls a grid pane renderer.
        self.grid.renderer.reset_trace();
        // `plan.consumes` is the attempt's taken `PendingWork`, owned by the
        // plan; moving it out here (rather than a second borrow of the
        // pre-take value) is what lets a held arm merge it straight back
        // into `self.pending`. An arm that commits does nothing further
        // with it; only an arm that *holds* merges its own narrowed retry
        // scope (or, for a whole-frame hold, this same value) back in.
        let overlay = plan.overlay;
        let work = plan.consumes;
        let result = match plan.grid {
            GridWork::None => self.paint_overlay_regime(model_dyn, &inputs),
            GridWork::Blit(blit_plan) => {
                self.paint_viewport_regime(model_dyn, &inputs, blit_plan, work)
            }
            GridWork::Panes(mask) => {
                self.paint_slots_reuse_regime(model_dyn, &inputs, mask, overlay)
            }
            GridWork::Fresh => self.paint_fresh_regime(model_dyn, &inputs, work),
            GridWork::Rows { sheet, spans } => {
                self.paint_damage_regime(model_dyn, &inputs, sheet, spans, overlay)
            }
        };

        self.last_trace = self.grid.renderer.trace();
        self.last_trace.regime = self.last_regime;
        self.last_trace.effective = self.last_effective;
        self.last_trace.work = self.last_work_flags;

        // Restore site for every regime that reached dispatch. The other
        // restore site is the capture-failure early return above, which
        // returns before any regime runs.
        self.model = Some(model);
        result
    }

    /// Retry requeue for a pane-local partial commit (`SlotsReuse`,
    /// `Fresh`): only the held panes' content comes back. Overlay work is
    /// deliberately not requeued — the overlay already painted and
    /// presented on this frame, so re-marking it would repaint identical
    /// pixels every tick until the bridge recovers.
    ///
    /// Merges rather than assigns: a producer may have queued new work
    /// while this paint ran, and assignment would silently drop it.
    fn requeue_held_panes(&mut self, held: PaneRegionMask) {
        let mut retry = PendingWork::default();
        retry.mark_panes(held);
        self.pending.merge(retry);
    }

    /// Retry requeue for a held `Damage` strip: the original sheet and row
    /// spans, so the next attempt keeps the band clipping instead of
    /// escalating to a whole-pane walk. Same merge-not-assign rule as
    /// [`Self::requeue_held_panes`].
    fn requeue_held_rows(&mut self, sheet: u32, spans: &[RowSpan]) {
        let mut retry = PendingWork::default();
        for span in spans {
            retry.mark_rows(sheet, *span);
        }
        self.pending.merge(retry);
    }

    /// Overlay-only fast path. Triggered by autofill drag, clipboard state
    /// change, formula-ref highlight updates, and active-cell moves —
    /// anything that leaves grid pixels untouched. `plan_frame` proves the
    /// preconditions (slot vecs still match, `last_frame` is `Some`).
    fn paint_overlay_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
    ) -> PaintResult {
        let Some(prev) = self.last_frame.as_ref() else {
            return PaintResult::Idle;
        };
        self.decos.refresh_overlay_state(
            model,
            inputs.sheet(),
            &inputs.view(),
            inputs.show_selection(),
        );
        self.overlay.paint_overlay_layer(
            model,
            prev,
            self.decos.selection(),
            &self.decos.overlay_slice(),
            self.decos.custom_layers(),
        );
        self.overlay.present();
        PaintResult::Painted
    }

    /// Scroll-blit fast path. `plan_frame` already filtered no-op scrolls and
    /// viewport shifts where the kept band can't be reused; we trust the
    /// verdict and the supplied plan. Always repaints the overlay too —
    /// a viewport shift moves every overlay primitive's pixel position.
    ///
    /// `Chrome::next_blit` may demote to `Fresh` when in-place reuse rejects
    /// (e.g. row-header digit boundary). The `BlitOutcome` variant we get back
    /// *is* the dispatch — the `FreshFallback` arm takes the full repaint with
    /// cache invalidation, instead of a `paint_grid_blit` that would carry
    /// stale per-pane caches against the freshly rebuilt slot vecs.
    fn paint_viewport_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        plan: BlitPlan,
        work: PendingWork,
    ) -> PaintResult {
        let Some(prev) = self.last_frame.take() else {
            return PaintResult::Idle;
        };
        // Held-restore snapshot: on a held preflight the screen still shows
        // `prev`'s pixels, so `prev` must return to `last_frame` untouched.
        // Deep-copies slot vecs + header labels per attempt; the Stage-4
        // prepare/commit split removes this clone.
        let restore = prev.clone();
        let frame = match Chrome::next_blit(Some(prev), model, inputs, &plan) {
            BlitOutcome::Blitted(frame) => {
                if matches!(
                    self.grid.paint_grid_blit(model, &frame, &plan),
                    BlitPaint::Held
                ) {
                    self.last_frame = Some(restore);
                    // Whole-frame hold: the preflight aborted before a
                    // pixel shifted, so nothing at all was committed and
                    // the entire attempt must come back — including the
                    // overlay mark, which never painted on this frame.
                    self.pending.merge(work);
                    return PaintResult::Retry;
                }
                frame
            }
            BlitOutcome::FreshFallback(frame) => {
                // `plan_frame` selected Viewport, but this arm actually did
                // a full repaint — the trace must attribute the frame to
                // what ran, not what was selected.
                self.last_effective = Some(PaintRegimeTag::Fresh);
                self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
                self.grid.invalidate_paint_cache();
                self.grid.paint_grid(model, &frame, PaneRegionMask::ALL);
                frame
            }
        };
        self.grid.present();
        self.decos.refresh_overlay_state(
            model,
            inputs.sheet(),
            &inputs.view(),
            inputs.show_selection(),
        );
        self.overlay.paint_overlay_layer(
            model,
            &frame,
            self.decos.selection(),
            &self.decos.overlay_slice(),
            self.decos.custom_layers(),
        );
        self.overlay.present();
        self.last_frame = Some(frame);
        PaintResult::Painted
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
        inputs: &FrameInputs,
        sheet: u32,
        spans: Vec<RowSpan>,
        overlay: OverlayWork,
    ) -> PaintResult {
        let Some(prev) = self.last_frame.take() else {
            return PaintResult::Idle;
        };
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);
        let held = self.grid.paint_grid_damage(model, &frame, &spans);
        self.grid.present();
        self.decos.refresh_overlay_state(
            model,
            inputs.sheet(),
            &inputs.view(),
            inputs.show_selection(),
        );
        // `plan_frame` already folded "content implies an active-cell
        // repaint" into `overlay` — this arm just reads the verdict rather
        // than re-deriving it from `PendingWork`/decoration state.
        if matches!(overlay, OverlayWork::Paint) {
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
        if !held.is_empty() {
            // Pane-local partial commit: the healthy bands painted and
            // presented, so only the original row scope returns. Requeued
            // as rows, not as the held pane mask — keeping the bands is
            // what lets the retry stay clipped instead of escalating to a
            // whole-pane walk. `sheet` is the original sheet `GridWork::Rows`
            // carried in from the plan — equal to `frame.sheet` (SlotsReuse
            // construction never changes the committed sheet), just sourced
            // from the plan instead of re-derived from the just-built frame.
            self.requeue_held_rows(sheet, &spans);
            return PaintResult::Retry;
        }
        PaintResult::Painted
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
        inputs: &FrameInputs,
        mask: PaneRegionMask,
        overlay: OverlayWork,
    ) -> PaintResult {
        let Some(prev) = self.last_frame.take() else {
            return PaintResult::Idle;
        };
        let frame = Chrome::next(Some(prev), model, inputs, FramePath::SlotsReuse);

        self.grid.invalidate_pane_cache(mask);
        self.grid.invalidate_paint_cache();

        let held = self.grid.paint_grid(model, &frame, mask);
        self.grid.present();
        // Refresh the selection snapshot unconditionally: even on a
        // content-only attempt the grid just repainted with new values,
        // so the next paint's `Chrome::classify` must compare against
        // the post-edit hash.
        self.decos.refresh_overlay_state(
            model,
            inputs.sheet(),
            &inputs.view(),
            inputs.show_selection(),
        );
        // `plan_frame` already folded "content implies an active-cell
        // repaint" into `overlay` (so DEL on the active cell still clears
        // the overlay's stale value even when only CONTENT was raised) —
        // this arm just reads the verdict rather than re-deriving it.
        if matches!(overlay, OverlayWork::Paint) {
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
        if !held.is_empty() {
            // Pane-local partial commit (see plan contract): painted panes
            // presented; held panes keep prior pixels. Retain failed scope.
            self.requeue_held_panes(held);
            return PaintResult::Retry;
        }
        PaintResult::Painted
    }

    /// Full grid repaint. Slot vecs walked fresh from the model; the new
    /// vecs make any cross-frame fingerprint compare meaningless, so every
    /// pane repaints. Selected when slot vecs diverged or no prior frame.
    ///
    /// Content work gates `PaneCache` invalidation: an edit escalated to
    /// Fresh (e.g. via a concurrent scroll) means the cache's range-matched
    /// buffers may now be stale against the new slot vecs. View work gates
    /// it for a distinct reason — pane-buffer ranges carry row/column
    /// coordinates but no sheet identity, so a view change that lands here
    /// because it switched the active sheet must not let a cached range be
    /// treated as describing the new sheet.
    ///
    /// Both clauses are belt-and-braces against the *cache*, not the fetch:
    /// `render_pane` bulk-fetches unconditionally on a non-slots-reuse
    /// frame, so neither clause changes what this frame reads from the
    /// model today. They exist so the invariant holds if a future stage
    /// teaches the Fresh path to adopt a range-matched buffer.
    fn paint_fresh_regime(
        &mut self,
        model: &dyn CanvasModel,
        inputs: &FrameInputs,
        work: PendingWork,
    ) -> PaintResult {
        let prev = self.last_frame.take();
        let frame = Chrome::next(prev, model, inputs, FramePath::Fresh);

        if work.has_content() || work.has_view() {
            self.grid.invalidate_pane_cache(PaneRegionMask::ALL);
        }
        self.grid.invalidate_paint_cache();
        let held = self.grid.paint_grid(model, &frame, PaneRegionMask::ALL);
        self.grid.present();
        self.decos.refresh_overlay_state(
            model,
            inputs.sheet(),
            &inputs.view(),
            inputs.show_selection(),
        );
        // Fresh always repaints the overlay (`plan_frame` never plans
        // `OverlayWork::Preserve` alongside `GridWork::Fresh`): candidate
        // geometry or model identity may have changed, so a preserved
        // overlay could show handles or a selection rect positioned against
        // pixels that no longer match.
        self.overlay.paint_overlay_layer(
            model,
            &frame,
            self.decos.selection(),
            &self.decos.overlay_slice(),
            self.decos.custom_layers(),
        );
        self.overlay.present();
        self.last_frame = Some(frame);
        if !held.is_empty() {
            self.requeue_held_panes(held);
            return PaintResult::Retry;
        }
        PaintResult::Painted
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
