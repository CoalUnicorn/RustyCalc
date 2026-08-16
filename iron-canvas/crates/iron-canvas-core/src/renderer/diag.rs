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

use crate::chrome::{GridLayout, GridShape, PaneRegion};
use crate::frame_plan::{FrameDelta, RebuildReason};
use crate::geometry::prim::Axis;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::CanvasSize;
use crate::orchestrator::{FrameOutcome, GridVerdict, PaintRegimeTag};
use crate::pending_work::{RowSpan, WorkFlags};
use crate::renderer::cache::BufferTruth;
use crate::renderer::cell::fingerprint::FingerprintTruth;
use crate::types::coord::RCRange;
/// Wire version of the snapshot shape. Bump when the projection changes.
pub const DIAG_SCHEMA_VERSION: u8 = 1;

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
    /// At least one row digest changed and the bands are paint-safe.
    ChangedRows,
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
#[derive(Clone, Debug, PartialEq)]
pub struct DiagGeometry {
    pub canvas: CanvasSize,
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
/// painting; `strip` is the newly exposed repaint band. The two are
/// distinct concepts that happen to share one value in today's finalized
/// blit work — the snapshot records the actual clip argument, not a
/// re-derivation of it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagBlit {
    pub axis: Axis,
    pub delta: i32,
    pub src: PixelRect,
    pub dst: PixelRect,
    pub clip: PixelRect,
    pub strip: PixelRect,
    pub revealed: Vec<DiagRevealedStrip>,
    pub result: DiagBlitResultTag,
    pub cold_cache: Option<bool>,
}

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
    /// this so an enable toggle mid-attempt never half-writes. Tasks 2-4
    /// add the section writers that call it; Task 1 publishes via
    /// `publish_diag` directly, hence the allow for now.
    #[allow(dead_code)]
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
    pub(crate) fn publish_diag(
        &self,
        attempt_seq: u64,
        selected: Option<PaintRegimeTag>,
        work: WorkFlags,
        effective: Option<PaintRegimeTag>,
        committed_seq: Option<u64>,
        outcome: FrameOutcome,
        layers: DiagPaintedLayers,
        resolution: DiagCacheResolution,
    ) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut snapshot = self.diag.capture.borrow_mut().take().unwrap_or_else(|| {
            FrameDiagnostics {
                schema_version: DIAG_SCHEMA_VERSION,
                ..FrameDiagnostics::default()
            }
        });
        snapshot.attempt_seq = attempt_seq;
        snapshot.committed_seq = committed_seq;
        snapshot.selected = selected;
        snapshot.effective = effective;
        snapshot.work = work;
        snapshot.outcome = outcome;
        snapshot.painted_layers = layers;
        snapshot.cache.resolution = resolution;
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
