//! The stable diagnostic snapshot vocabulary.
//!
//! Every type here is an evidence fact a consumer may name without knowing
//! how the renderer collects it. Collection — the in-flight buffer, private
//! prepared-data reads, and cache sampling — stays in the renderer's `diag`
//! module, behind the `dev-diagnostics` feature; these definitions carry no
//! renderer state.
//!
//! The types refer to core ranges, layouts, strategies, and outcomes. They
//! stay in this crate: a separate crate depending on core while core depends
//! on it would be a dependency cycle.

use crate::chrome::{GridLayout, GridShape, PaneRegion};
use crate::frame_plan::{FrameDelta, RebuildReason};
use crate::geometry::CanvasSize;
use crate::geometry::pixel_rect::PixelRect;
use crate::geometry::prim::Axis;
use crate::orchestrator::{FrameOutcome, GridVerdict, RenderStrategy};
use crate::pending_work::{RowSpan, WorkFlags};
use crate::types::coord::RCRange;

/// Wire version of the snapshot shape. Bump when the projection changes.
/// Schema 3 replaced `overlay`, `viewport`, `slotsReuse`, `fresh`, and
/// `damage` with the camelCase `RenderStrategy` names `overlayOnly`,
/// `scrollBlit`, `changedCells`, `fullRebuild`, and `damagedRows`.
/// Schema 4 removed the `probe` and `probeSegments` fields.
pub const DIAG_SCHEMA_VERSION: u8 = 4;

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
///
/// This public mirror is intentionally separate from the private planner
/// enum: snapshot names are a stable diagnostics contract, while planner
/// internals may evolve independently. Every variant here must have a live
/// producer in the exhaustive conversion in `diag_repaint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagRepaintReason {
    /// `FingerprintState::painted` was `None` — first content comparison.
    NoPaintedHistory,
    /// Committed and candidate layouts (or row counts) differ.
    LayoutMismatch,
    /// Row addresses diverged at some absolute row.
    RowAddressMismatch,
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

/// Prepared grid-cache transition tag.
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

/// Stable diagnostic mirror of renderer cache-buffer truth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiagBufferTruth {
    Valid,
    #[default]
    Stale,
}

/// Stable diagnostic mirror of renderer fingerprint truth.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiagFingerprintTruth {
    Exact,
    #[default]
    Stale,
}

/// What happened to the prepared cache action. Derived from the TRANSACTION
/// outcome, never from the presence of a grid cache commit: an Overlay
/// strategy commits with `cache_commit: None`. There is no "discarded" state
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
/// size and DPR via [`CanvasMetrics::backing_size`](crate::CanvasMetrics::backing_size)
/// (browser rounding).
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

/// Blit geometry for a `ScrollBlit` attempt. `delta` is the logical row or
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
    pub selected: Option<RenderStrategy>,
    pub effective: Option<RenderStrategy>,
    pub work: WorkFlags,
    pub delta: Option<DiagDeltaKind>,
    pub rebuild_reason: Option<RebuildReason>,
    pub outcome: FrameOutcome,
    pub painted_layers: DiagPaintedLayers,
    pub geometry: Option<DiagGeometry>,
    pub fetch: DiagFetch,
    pub repaint: DiagRepaint,
    pub cache: DiagCache,
    pub blit: Option<DiagBlit>,
    pub paint_counts: DiagPaintCounts,
}
