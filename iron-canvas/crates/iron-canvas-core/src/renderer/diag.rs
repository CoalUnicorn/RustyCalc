//! Structured per-attempt diagnostics for dev builds.
//!
//! `FrameTrace` answers "which path painted this frame?" in one allocation-
//! free line. This module answers "why?" with a typed snapshot of the same
//! attempt: planned segments, the host's probe address and which segments
//! contain it, renderer-owned fetch requests, the repaint decision and its
//! reason, the prepared/committed cache transition, blit geometry, and
//! painted row/cell counts.
//!
//! Capture is a pure observer: nothing here re-runs classifiers, changes
//! planner outcomes, or touches committed cache state. All writes are
//! feature-gated (`dev-diagnostics`) and no-ops while `enabled` is false,
//! so disabled capture performs no allocations. Wall-clock reads belong to
//! the host; core never samples a clock here.
//!
//! The grid `RendererCore` owns one `DiagState`: an in-flight `capture`
//! buffer written during prepare/execute, and a `published` last snapshot
//! moved there only by `Orchestrator::finish_attempt` — so a held attempt
//! can never surface candidate layout or cache state as committed.

use std::cell::{Cell, RefCell};

use crate::chrome::BlitPlan;
use crate::chrome::Chrome;
use crate::chrome::{GridLayout, GridShape, PaneRegion};
use crate::frame_plan::{FrameDelta, RebuildReason};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::orchestrator::{FrameOutcome, GridVerdict, PaintRegimeTag};
use crate::pending_work::{RowSpan, WorkFlags};
use crate::renderer::cache::BufferTruth;
use crate::renderer::cell::fingerprint::{FingerprintTruth, RepaintReason};
use crate::renderer::prepared::FetchedCells;
use crate::types::coord::RCRange;
/// Wire version of the snapshot shape. Bump when the projection changes.
pub const DIAG_SCHEMA_VERSION: u8 = 2;

/// Classification verdict for this attempt, as `Chrome::classify` decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagDeltaKind {
    Stable,
    Scroll,
    Rebuild,
}

impl From<&FrameDelta> for DiagDeltaKind {
    fn from(delta: &FrameDelta) -> Self {
        match delta {
            FrameDelta::Stable => Self::Stable,
            FrameDelta::Scroll(_) => Self::Scroll,
            FrameDelta::Rebuild(_) => Self::Rebuild,
        }
    }
}

/// Why one renderer-owned bundle fetch was issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagFetchPurpose {
    /// Whole-segment fetch during a full-grid prepare.
    FullSegment,
    /// Full-width row band for a Damage repaint.
    DamageStrip,
    /// Newly revealed address strip for a scroll blit.
    BlitReveal,
}

/// Why the fingerprint comparison reached its verdict. Names ONLY branches
/// the comparison itself took — a Fresh-built geometry or a Damage/Blit
/// strip never runs the comparison, so they carry no reason and the
/// captured `rebuild_reason` (or the strategy) is their authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagRepaintReason {
    /// `FingerprintState::painted` was `None` — first content comparison.
    NoPaintedHistory,
    /// Committed and candidate layouts (or row counts) differ.
    LayoutMismatch,
    /// Row addresses diverged at some absolute row.
    RowAddressMismatch,
    /// More than 8 disjoint changed bands — whole-grid walk wins.
    SpanCapExceeded,
    /// A changed band's edge row carries an explicit border.
    BorderSafety,
    /// Every compared row digest matched — nothing to paint.
    FingerprintsEqual,
    /// Exactly one retained cell leaf changed.
    ChangedCell,
    /// Several retained cell leaves selected one merged envelope.
    ChangedCells,
    /// At least one row digest changed and the bands are paint-safe.
    ChangedRows,
    /// An integer-CSS clip could not be aligned to backing pixels.
    ClipAlignment,
}

/// Prepared grid-cache action tag (projection of `GridCacheAction`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagCacheActionTag {
    None,
    Replace,
    Splice,
    Shift,
    Reset,
}

/// Fingerprint update carried by the prepared commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagFingerprintActionTag {
    Install,
    MarkStale,
    Reset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiagBufferTruth {
    Valid,
    #[default]
    Stale,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiagFingerprintTruth {
    Exact,
    #[default]
    Stale,
}

/// What happened to the prepared cache action. Derived from the TRANSACTION
/// outcome, never from the presence of a grid cache commit: an Overlay
/// regime commits with `cache_commit: None`. There is no "discarded" state
/// in the current pipeline — an attempt either commits or holds whole-grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagCacheResolution {
    Committed,
    HeldForRetry,
}

/// How the blit attempt resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagBlitResultTag {
    /// Compatible shift: kept band blitted, revealed strips repainted.
    Shifted,
    /// A revealed-strip bridge fetch failed; whole attempt held.
    HeldPreflight,
    /// In-renderer fallback: layout/buffer preconditions failed, the
    /// frame was prepared as a full-grid replacement.
    GridFallback,
    /// `Chrome::prepare_blit` rejected in-place reuse; full Fresh rebuild.
    FreshFallback,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiagPaintedLayers {
    pub grid: bool,
    pub overlay: bool,
}

/// One populated visible address segment, canonical TL/TR/BL/BR order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagSegment {
    pub region: PaneRegion,
    pub range: RCRange,
    pub cells: usize,
}

/// Geometry facts of the frame the grid prepared against. `None` for an
/// overlay-only attempt (the grid renderer was never entered).
///
/// `backing_size` is the physical backing-store size derived from the CSS
/// size and DPR via [`CanvasSize::to_backing_size`] (browser rounding).
/// Core never sees the backend canvas element, so this is the documented
/// derivation; the web facade overwrites it with the actual canvas
/// backing store when the snapshot is projected, making CSS/backing
/// mismatches visible.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagGeometry {
    pub canvas: CanvasSize,
    pub backing_size: (u32, u32),
    pub dpr: f64,
    pub sheet: u32,
    pub top_row: i32,
    pub left_column: i32,
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    pub show_row_headers: bool,
    pub show_col_headers: bool,
    pub shape: GridShape,
    pub segments: Vec<DiagSegment>,
}

/// One renderer-owned bundle fetch. Renderer requests, not host or engine
/// call counts — an adapter may satisfy one bundle with many scalar reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagFetchRequest {
    pub purpose: DiagFetchPurpose,
    pub region: Option<PaneRegion>,
    pub range: RCRange,
    pub cells: usize,
    pub slots: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagFetch {
    pub batches: usize,
    pub addressed_cells: usize,
    pub logical_slots: usize,
    pub requests: Vec<DiagFetchRequest>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagRepaint {
    pub verdict: Option<GridVerdict>,
    pub reason: Option<DiagRepaintReason>,
    pub changed_rows: Vec<RowSpan>,
    pub changed_cells: Vec<DiagChangedCell>,
    pub clip: Option<PixelRect>,
    pub source_ranges: Vec<DiagSourceRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagChangedCell {
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagSourceRange {
    pub region: PaneRegion,
    pub range: RCRange,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiagCacheTruth {
    pub layout: Option<GridLayout>,
    pub buffer_truth: DiagBufferTruth,
    pub fingerprint_truth: DiagFingerprintTruth,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagCache {
    pub planned_action: Option<DiagCacheActionTag>,
    pub fingerprint_action: Option<DiagFingerprintActionTag>,
    /// Committed truth sampled at attempt start (before any prepare).
    /// `None` only for a capture-failure attempt, which never sampled it —
    /// publication then fills it equal to `committed_after`, because a
    /// capture failure precedes every cache interaction.
    pub committed_before: Option<DiagCacheTruth>,
    pub resolution: DiagCacheResolution,
    pub committed_after: DiagCacheTruth,
}

impl Default for DiagCache {
    fn default() -> Self {
        Self {
            planned_action: None,
            fingerprint_action: None,
            committed_before: None,
            resolution: DiagCacheResolution::Committed,
            committed_after: DiagCacheTruth::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagRevealedStrip {
    pub region: PaneRegion,
    pub range: RCRange,
}

/// Blit geometry for a `Viewport` attempt. `delta` is the logical row or
/// column count the viewport moved (negative = toward the origin). `clip`
/// is the exact pixel rectangle `Painter::push_clip` applied around strip
/// painting, `Some` only when execution actually reached `push_clip` —
/// fallback and held attempts never apply a clip and report `None`, never
/// a fabricated zero rectangle. `strip` is the newly exposed repaint band.
/// The two are distinct concepts that happen to share one value in today's
/// finalized blit work — the snapshot records the actual clip argument,
/// not a re-derivation of it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagBlit {
    pub axis: Axis,
    pub delta: i32,
    pub src: PixelRect,
    pub dst: PixelRect,
    pub clip: Option<PixelRect>,
    pub strip: PixelRect,
    pub revealed: Vec<DiagRevealedStrip>,
    pub result: DiagBlitResultTag,
    pub cold_cache: Option<bool>,
}

/// Painted-area accounting. `rows` counts DISTINCT absolute grid rows
/// painted by this attempt, deduplicated across segments — frozen columns
/// split one row band into left/right segments, and a one-row repaint must
/// report one row, not one per visited segment. `cells` counts addressed
/// cells; segments are column-disjoint, so cells never double-count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiagPaintCounts {
    pub rows: usize,
    pub cells: usize,
}

/// Structured snapshot of one completed live paint attempt. Published by
/// `finish_attempt` only; serde projection lives in `iron-canvas-web`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameDiagnostics {
    pub schema_version: u8,
    pub attempt_seq: u64,
    pub committed_seq: Option<u64>,
    pub selected: Option<PaintRegimeTag>,
    pub effective: Option<PaintRegimeTag>,
    pub work: WorkFlags,
    pub delta: Option<DiagDeltaKind>,
    pub rebuild_reason: Option<RebuildReason>,
    pub outcome: FrameOutcome,
    pub painted_layers: DiagPaintedLayers,
    /// Host-supplied expected-change address, latched by this attempt.
    /// Diagnostic evidence only — never read by the planner.
    pub probe: Option<RCRange>,
    /// Segments whose `RCRange` fully contains `probe`. Empty when the
    /// probe lies outside every planned segment or no probe was set.
    pub probe_segments: Vec<PaneRegion>,
    pub geometry: Option<DiagGeometry>,
    pub fetch: DiagFetch,
    pub repaint: DiagRepaint,
    pub cache: DiagCache,
    pub blit: Option<DiagBlit>,
    pub paint_counts: DiagPaintCounts,
}

/// Frame-completion facts assembled by `Orchestrator::finish_attempt` and
/// handed to publication as ONE value. The renderer wrapper and the core
/// sink take this by value so adjacent scalar arguments cannot be swapped
/// at the wrapper boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagCompletion {
    pub attempt_seq: u64,
    pub selected: Option<PaintRegimeTag>,
    pub work: WorkFlags,
    pub effective: Option<PaintRegimeTag>,
    pub committed_seq: Option<u64>,
    pub outcome: FrameOutcome,
    pub layers: DiagPaintedLayers,
    pub resolution: DiagCacheResolution,
}

/// Renderer-owned capture state: enable flag, in-flight buffer, published
/// last snapshot. Interior mutability because paint methods run on `&self`.
pub(crate) struct DiagState {
    enabled: Cell<bool>,
    capture: RefCell<Option<FrameDiagnostics>>,
    published: RefCell<Option<FrameDiagnostics>>,
}

impl Default for DiagState {
    fn default() -> Self {
        Self {
            enabled: Cell::new(false),
            capture: RefCell::new(None),
            published: RefCell::new(None),
        }
    }
}

impl DiagState {
    /// Empty the in-flight capture. Called by `paint_if_dirty` at attempt
    /// start so a capture-failure hold cannot inherit the previous
    /// attempt's renderer sections.
    pub(crate) fn reset_capture(&self) {
        *self.capture.borrow_mut() = None;
    }

    /// `&mut` access to a fresh capture. All write sites route through
    /// this so an enable toggle mid-attempt never half-writes.
    fn ensure_capture(&self) -> std::cell::RefMut<'_, Option<FrameDiagnostics>> {
        let mut slot = self.capture.borrow_mut();
        slot.get_or_insert_with(|| FrameDiagnostics {
            schema_version: DIAG_SCHEMA_VERSION,
            ..FrameDiagnostics::default()
        });
        slot
    }
}

#[cfg(feature = "dev-diagnostics")]
impl<P: crate::painter::Painter> crate::renderer::RendererCore<P> {
    /// Committed cache truth at the moment of the read. Shared by the
    /// attempt-start sample (`diag_begin_attempt`, Task 2) and the
    /// post-install read in `publish_diag`.
    fn cache_truth_now(&self) -> DiagCacheTruth {
        DiagCacheTruth {
            layout: self.grid_cache.layout(),
            buffer_truth: if self.grid_cache.buffer_truth() == BufferTruth::Valid {
                DiagBufferTruth::Valid
            } else {
                DiagBufferTruth::Stale
            },
            fingerprint_truth: match self.grid_cache.fingerprint.truth() {
                FingerprintTruth::Exact => DiagFingerprintTruth::Exact,
                FingerprintTruth::Stale => DiagFingerprintTruth::Stale,
            },
        }
    }

    /// Classification facts plus the attempt-scoped probe and the
    /// attempt-start committed cache truth, recorded once by
    /// `paint_if_dirty` after `plan_frame`. The capture-failure path never
    /// calls this — its snapshot keeps `delta`/`rebuild_reason`/`probe` at
    /// `None` and `committed_before` is filled by `publish_diag`.
    pub(crate) fn diag_begin_attempt(
        &self,
        delta: DiagDeltaKind,
        rebuild_reason: Option<RebuildReason>,
        probe: Option<RCRange>,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.capture.borrow_mut();
        *slot = Some(FrameDiagnostics {
            schema_version: DIAG_SCHEMA_VERSION,
            delta: Some(delta),
            rebuild_reason,
            probe,
            cache: DiagCache {
                committed_before: Some(self.cache_truth_now()),
                ..DiagCache::default()
            },
            ..FrameDiagnostics::default()
        });
    }

    /// Geometry of the frame a grid prepare is about to paint against,
    /// plus which planned segments fully contain the attempt's probe.
    /// Called from every grid prepare entry point; idempotent overwrite.
    pub(crate) fn diag_geometry(&self, frame: &Chrome, layout: GridLayout) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.probe_segments = capture
            .probe
            .into_iter()
            .flat_map(|probe| {
                layout.segments().filter_map(move |segment| {
                    let range = segment.range();
                    (range.r1 <= probe.r1
                        && range.c1 <= probe.c1
                        && range.r2 >= probe.r2
                        && range.c2 >= probe.c2)
                        .then_some(segment.region())
                })
            })
            .collect();
        capture.geometry = Some(DiagGeometry {
            canvas: frame.canvas_size,
            backing_size: frame.canvas_size.to_backing_size(frame.dpr),
            dpr: frame.dpr,
            sheet: frame.sheet,
            top_row: frame.pane_set.top_row(),
            left_column: frame.pane_set.left_column(),
            row_header_thickness: frame.row_header_thickness,
            col_header_thickness: frame.col_header_thickness,
            show_row_headers: frame.show_row_headers,
            show_col_headers: frame.show_col_headers,
            shape: layout.shape(),
            segments: layout
                .segments()
                .map(|segment| DiagSegment {
                    region: segment.region(),
                    range: segment.range(),
                    cells: FetchedCells::addressed_cells(segment.range()),
                })
                .collect(),
        });
    }

    /// One renderer-owned bundle fetch over `range`. Mirrors the existing
    /// `trace_fetch` counters with per-request attribution.
    pub(crate) fn diag_fetch(
        &self,
        purpose: DiagFetchPurpose,
        region: Option<PaneRegion>,
        range: RCRange,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.fetch.batches += 1;
        capture.fetch.addressed_cells += FetchedCells::addressed_cells(range);
        capture.fetch.logical_slots += FetchedCells::logical_channel_slots(range);
        capture.fetch.requests.push(DiagFetchRequest {
            purpose,
            region,
            range,
            cells: FetchedCells::addressed_cells(range),
            slots: FetchedCells::logical_channel_slots(range),
        });
    }

    /// Grid verdict plus the fingerprint branch reason (when a comparison
    /// ran) and the exact changed row spans. The single recorder for all
    /// three grid arms — Full (with comparison reason), Damage and Blit
    /// (both `Strip`, no reason) — so the structured snapshot never
    /// disagrees with the one-line trace.
    pub(crate) fn diag_repaint(
        &self,
        verdict: GridVerdict,
        reason: Option<RepaintReason>,
        changed_rows: &[RowSpan],
        changed_cells: &[RCRange],
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.repaint.verdict = Some(verdict);
        capture.repaint.reason = reason.map(|reason| match reason {
            RepaintReason::NoPaintedHistory => DiagRepaintReason::NoPaintedHistory,
            RepaintReason::LayoutMismatch => DiagRepaintReason::LayoutMismatch,
            RepaintReason::RowAddressMismatch => DiagRepaintReason::RowAddressMismatch,
            RepaintReason::FingerprintsEqual => DiagRepaintReason::FingerprintsEqual,
            RepaintReason::ChangedCell => DiagRepaintReason::ChangedCell,
            RepaintReason::ChangedCells => DiagRepaintReason::ChangedCells,
            RepaintReason::ChangedRows => DiagRepaintReason::ChangedRows,
            RepaintReason::ClipAlignment => DiagRepaintReason::ClipAlignment,
        });
        capture.repaint.changed_rows = changed_rows.to_vec();
        capture.repaint.changed_cells = changed_cells
            .iter()
            .map(|cell| {
                let cell = cell.normalized();
                DiagChangedCell {
                    row: cell.r1,
                    column: cell.c1,
                }
            })
            .collect();
    }

    pub(crate) fn diag_repaint_envelope(
        &self,
        clip: Option<PixelRect>,
        sources: &[Option<RCRange>; 4],
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.repaint.clip = clip;
        capture.repaint.source_ranges = [
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight,
        ]
        .into_iter()
        .filter_map(|region| {
            sources[region as usize].map(|range| DiagSourceRange { region, range })
        })
        .collect();
    }

    /// Prepared grid-cache action tag, recorded once by each prepare entry
    /// point next to its `cache_action`.
    pub(crate) fn diag_cache_planned(&self, action: DiagCacheActionTag) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.cache.planned_action = Some(action);
    }

    /// Fingerprint update carried by the prepared commit, recorded at each
    /// execute arm where the commit installs it.
    pub(crate) fn diag_fingerprint_action(&self, action: DiagFingerprintActionTag) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.cache.fingerprint_action = Some(action);
    }

    /// Blit detail for a `Viewport` attempt. `delta` is derived from the
    /// committed vs candidate scroll-band origin — the renderer never
    /// re-reads the model for it. `clip` is recorded separately at the
    /// `push_clip` call site, not derived here.
    pub(crate) fn diag_blit(
        &self,
        plan: &BlitPlan,
        result: DiagBlitResultTag,
        cold_cache: Option<bool>,
        previous: Option<GridLayout>,
        candidate: GridLayout,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let delta = match (previous, plan.axis) {
            (Some(previous), Axis::Row) => scroll_delta(previous, candidate, true),
            (Some(previous), Axis::Column) => scroll_delta(previous, candidate, false),
            (None, _) => 0,
        };
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.blit = Some(DiagBlit {
            axis: plan.axis,
            delta,
            src: plan.shift.src,
            dst: plan.shift.dst,
            // `None` until the execute arm actually reaches `push_clip`;
            // fallback and held attempts never apply a clip.
            clip: None,
            strip: plan.pixel_strip,
            revealed: Vec::new(),
            result,
            cold_cache,
        });
    }

    pub(crate) fn diag_blit_revealed(&self, region: PaneRegion, range: RCRange) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        if let Some(blit) = &mut capture.blit {
            blit.revealed.push(DiagRevealedStrip { region, range });
        }
    }

    /// The exact pixel rectangle handed to `Painter::push_clip` for strip
    /// painting — the effective grid clip, recorded at the call site.
    pub(crate) fn diag_blit_clip(&self, clip: PixelRect) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        if let Some(blit) = &mut capture.blit {
            blit.clip = Some(clip);
        }
    }

    pub(crate) fn diag_paint_counts(&self, rows: usize, cells: usize) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.paint_counts.rows += rows;
        capture.paint_counts.cells += cells;
    }

    /// Runtime switch. Disabling drops the retained published snapshot so
    /// the web facade's `frameDiagnostics()` returns `undefined`.
    pub(crate) fn set_diag_enabled(&self, enabled: bool) {
        self.diag.enabled.set(enabled);
        if !enabled {
            *self.diag.published.borrow_mut() = None;
            *self.diag.capture.borrow_mut() = None;
        }
    }

    pub(crate) fn diag_reset_capture(&self) {
        self.diag.reset_capture();
    }

    /// Seal the in-flight capture and move it into `published`. Only
    /// `Orchestrator::finish_attempt` calls this, after the cache commit
    /// (if any) was installed — so `committed_after` reads the
    /// post-commit truth and a held attempt keeps
    /// `committed_before == committed_after`.
    pub(crate) fn publish_diag(&self, completion: DiagCompletion) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut snapshot =
            self.diag
                .capture
                .borrow_mut()
                .take()
                .unwrap_or_else(|| FrameDiagnostics {
                    schema_version: DIAG_SCHEMA_VERSION,
                    ..FrameDiagnostics::default()
                });
        snapshot.attempt_seq = completion.attempt_seq;
        snapshot.committed_seq = completion.committed_seq;
        snapshot.selected = completion.selected;
        snapshot.effective = completion.effective;
        snapshot.work = completion.work;
        snapshot.outcome = completion.outcome;
        snapshot.painted_layers = completion.layers;
        snapshot.cache.resolution = completion.resolution;
        let committed_after = self.cache_truth_now();
        // A capture-failure attempt never reaches a grid prepare, so its
        // committed cache could not have changed during the attempt —
        // before == after by construction.
        if snapshot.cache.committed_before.is_none() {
            snapshot.cache.committed_before = Some(committed_after.clone());
        }
        snapshot.cache.committed_after = committed_after;
        *self.diag.published.borrow_mut() = Some(snapshot);
    }

    /// Clone of the last published snapshot. Called by the web facade on
    /// demand only.
    pub(crate) fn last_diag(&self) -> Option<FrameDiagnostics> {
        self.diag.published.borrow().clone()
    }
}

/// Number of distinct absolute grid rows covered by the given inclusive
/// `(r1, r2)` intervals. Intervals from different segments overlap when
/// frozen columns split one band into left and right halves; merging them
/// keeps `DiagPaintCounts.rows` a unique-row count. Intervals are assumed
/// valid (`r1 <= r2`), as constructed by the execute arms.
#[cfg(feature = "dev-diagnostics")]
pub(crate) fn distinct_rows(intervals: &[(i32, i32)]) -> usize {
    let mut sorted: Vec<(i32, i32)> = intervals.to_vec();
    sorted.sort_unstable_by_key(|(r1, _)| *r1);
    let mut rows = 0usize;
    let mut current: Option<(i32, i32)> = None;
    for (r1, r2) in sorted {
        match current {
            Some((c1, c2)) if r1 <= c2 + 1 => {
                current = Some((c1, c2.max(r2)));
            }
            Some((c1, c2)) => {
                rows += (c2 - c1 + 1) as usize;
                current = Some((r1, r2));
            }
            None => current = Some((r1, r2)),
        }
    }
    if let Some((c1, c2)) = current {
        rows += (c2 - c1 + 1) as usize;
    }
    rows
}

/// Logical origin delta of the scroll-band BottomRight segment along the
/// given axis (`true` = rows). Zero when either side lacks the segment —
/// the blit result tag still carries the real fallback cause.
fn scroll_delta(previous: GridLayout, candidate: GridLayout, rows: bool) -> i32 {
    let origin = |layout: GridLayout| {
        layout
            .segments()
            .find(|segment| segment.region() == PaneRegion::BottomRight)
            .map(|segment| {
                let range = segment.range();
                if rows { range.r1 } else { range.c1 }
            })
    };
    match (origin(previous), origin(candidate)) {
        (Some(before), Some(after)) => after - before,
        _ => 0,
    }
}
