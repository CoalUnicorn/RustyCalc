# Granular Live Frame Diagnostics — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional, structured, runtime-enabled diagnostic snapshot of each live paint attempt to dev builds of `iron-canvas`, exposed through `dev-tools`-only JS methods and an expanded RustyCalc Perf panel, so a developer can explain any suspicious `frameTrace()` line (segments, fetches, repaint decisions, cache transitions, blit geometry) without re-running classifiers.

**Architecture:** A new internal `dev-diagnostics` feature in `iron-canvas-core` (enabled only by `iron-canvas-web/dev-tools` and core's self dev-dependency) gates all capture state. The grid `RendererCore` owns a `DiagState` (enable flag + in-flight capture buffer + published last snapshot); `Orchestrator::finish_attempt` is the only publisher. Segment attribution uses an attempt-scoped, range-only, host-supplied probe address: the capture reports which exact segments contain it — it never enters planner eligibility and carries no cell contents. The web crate owns the serde wire projection; RustyCalc drives the toggle through the existing one-shot-command pattern (`AppState` command drained by a worksheet Effect that pushes the canvas setter and pokes the rAF loop).

**Tech Stack:** Rust 2024 edition workspace (`iron-canvas` crates), wasm-bindgen + serde-wasm-bindgen, Leptos 0.8 (`leptos-use` 0.19) for RustyCalc, wasm-bindgen-test + ChromeDriver for browser gates.

**Spec:** `iron-canvas/docs/designs/2026-08-16-granular-live-frame-diagnostics.md` (design doc)

**Review:** `iron-canvas/docs/reviews/2026-08-16-granular-live-frame-diagnostics-plan-review.md` — all findings incorporated (see Self-Review Notes at the end).

## Global Constraints

- Public build switch stays `dev-tools`. New internal feature `dev-diagnostics` in `iron-canvas-core`, enabled by `iron-canvas-web/dev-tools` and by core's self dev-dependency. NEVER gate on `debug_assertions` (Task 4 timing probes need `--release --features dev-tools`).
- `FrameTrace` stays compact, `Copy`, allocation-free — the one-line Perf panel summary is untouched. Do not add tokens to its `Display`.
- Detailed capture is runtime-disabled by default; when disabled it performs no allocations, no vector pushes, and no clock reads. Core performs NO wall-clock reads at all — host wall time stays at the RustyCalc rAF boundary (`perf::now()` around `paint_if_dirty`).
- Diagnostics observe decisions only: no change to planner eligibility, fingerprint comparison outcomes, `RepaintPlan`, `GridVerdict`, cache commit semantics, or raster behavior. No re-running classifiers.
- The probe address (`probe`) is attempt-scoped and range-only: latched by the next non-idle paint attempt, cleared on consumption, never read by `plan_frame`/`Chrome::classify`/any prepare path, never carrying cell contents. It exists solely to answer "which segment contains the address the host expected to change".
- No cell values, formatted text, formulas, or fingerprint hashes may appear in any captured field or wire shape. Ranges, counts, enum tags, and pixel rectangles only.
- A held attempt must never present candidate layout/cache state as committed: `cache.committedBefore == cache.committedAfter` and `resolution: heldForRetry` on every held outcome. A committed Overlay attempt must report `resolution: committed` — cache-work presence is NOT transaction outcome (Overlay commits with `cache_commit: None`). `finish_attempt` is the only publication site.
- `DiagRepaintReason` names only branches the fingerprint comparison actually took. Fresh-built geometry and Damage/Blit strips report no fingerprint reason (`reason: null`); the captured `rebuildReason` is the authority for rebuilds.
- Damage and Blit attempts report `GridVerdict::Strip` in the structured snapshot, matching the one-line trace on all five strategies.
- Recorder `.icr` schema (v5) is untouched in this plan. `recordingCurrentAttempt()` behavior is untouched.
- Core keeps `FrameDiagnostics` serde-free; `iron-canvas-web/src/wire.rs` owns the wire projection (camelCase, kind-tagged enums, `schemaVersion: 1`). One reusable wire mirror, conversion-tested natively before browser scenarios use it.
- Adopted answers to the design's open questions: last snapshot only; runtime-disabled; expandable JSON view + copy action (no structured section rendering in v1); painted row/cell counts now (derived at the renderer boundary, no per-cell overhead); primitive-op counts deferred; cache truth gets diag-only names (`valid/stale`, `exact/stale`); no recorder schema change.
- Every task ends with `cargo test -p iron-canvas-core --locked` green (and the browser/wasm gates where stated). Each task's test gate must pass with that task's code alone — no future-task facts. Commit after each task with a concise, technical message.

## File Map (locked against working tree, 2026-08-16)

**`iron-canvas/crates/iron-canvas-core/`**
- `Cargo.toml` — add `dev-diagnostics = []`; self dev-dependency gains `"dev-diagnostics"`.
- `src/renderer/diag.rs` — CREATE: all diagnostic domain types + `DiagState` + `RendererCore` capture/publish methods.
- `src/renderer/mod.rs` — `RendererCore` gains cfg-gated `pub(crate) diag: diag::DiagState` field; module declaration; `for_layer` init.
- `src/renderer/prepared.rs` — capture call sites in `prepare_full_grid` (:294), `prepare_damage_grid` (:350), `prepare_blit_grid` (:407), `execute_prepared_grid` (:505); `PreparedRepaint` gains `reason: Option<RepaintReason>`.
- `src/renderer/cell/fingerprint.rs` — `RepaintReason` enum; `RepaintDecision { plan, reason }`; `compare_to_painted` (:281) and `plan_grid_repaint` (:403) return it; `FingerprintState::truth()` accessor; unit tests updated (:814-840).
- `src/renderer/cache/grid_cache.rs` — no change needed (`layout()`, `buffer_truth()`, `pub(crate) fingerprint` already accessible).
- `src/orchestrator.rs` — `set_frame_diagnostics_enabled` / `frame_diagnostics` / `set_frame_diagnostics_probe` + `diag_probe` field (cfg-gated); `diag_begin_attempt` call with probe latch in `paint_if_dirty` (:1105); `publish_diag` call in `finish_attempt` (:1254); blit result tagging in `paint_viewport_regime` (:1436) and `paint_fresh_fallback` (:1512).
- `src/lib.rs` — `#[cfg(feature = "dev-diagnostics")] pub use renderer::diag::{FrameDiagnostics, …};`
- `tests/diagnostics.rs` — CREATE: native integration tests (harness: `TestModel` + `Orchestrator<MemSurface>`, pattern from `tests/orchestrator_regimes.rs:40`).

**`iron-canvas/crates/iron-canvas-web/`**
- `Cargo.toml` — `dev-tools` adds `"iron-canvas-core/dev-diagnostics"`.
- `src/wire.rs` — `FrameDiagnosticsWire` + child shapes + `From<&FrameDiagnostics>` + a native `#[cfg(test)]` conversion test (cfg `dev-tools`).
- `src/orchestrator.rs` — `setFrameDiagnosticsEnabled` / `frameDiagnostics` / `setFrameDiagnosticsProbe` facade methods (cfg `dev-tools`), next to `frame_trace()` (:255) / `recording_current_attempt()` (:267).
- `tests/render_wasm.rs` — new `stable_diag_canvas_over` helper + dev-diagnostics browser cases; existing `stage6_*` helpers reused.

**RustyCalc (repo root)**
- `src/app_state.rs` — `diag_cmd: Split<Option<bool>>` one-shot (both flavors, like `RecordingCmd`).
- `src/perf.rs` — `PerfTimings` gains `diag_enabled: RwSignal<bool>` (authoritative canvas state) and `frame_diagnostics: RwSignal<Option<String>>`.
- `src/components/workbook/worksheet/dev_tools_effects.rs` — `install_diag_effect(state, app, canvas_handle, poke)`.
- `src/components/workbook/worksheet/mod.rs` — install the effect next to the others (:165-169).
- `src/components/workbook/worksheet/raf_loop.rs` — snapshot sampling only (reads signals; no closure mutation).
- `src/components/panels/perf_panel.rs` — toggle button + `Popover` JSON view + copy button + `on_cleanup` force-off (cfg `dev-tools`).
- `styles/panels/perf-panel.css` — styles for button/JSON/copy + the `position: fixed` popover surface.

**Docs**
- `iron-canvas/ARCHITECTURE.md` — new "Live frame diagnostics" subsection; bump header dates.
- `iron-canvas/README.md` — dev-tools section (:354-365) lists the three new methods.
- `iron-canvas/docs/designs/2026-08-16-granular-live-frame-diagnostics.md` — status → Implemented; "Questions to settle" → recorded decisions.
- `iron-canvas/docs/performance/2026-08-16-task4-probe-discipline.md` — CREATE: probe runbook.

---

### Task 1: Core `dev-diagnostics` feature, domain types, runtime switch, transaction-truth publish

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-core/Cargo.toml`
- Create: `iron-canvas/crates/iron-canvas-core/src/renderer/diag.rs`
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/mod.rs` (module decl + field + `for_layer` init)
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/cell/fingerprint.rs` (`FingerprintState::truth()`)
- Modify: `iron-canvas/crates/iron-canvas-core/src/orchestrator.rs` (setter/getter + publish call in `finish_attempt`)
- Modify: `iron-canvas/crates/iron-canvas-core/src/lib.rs` (re-export)
- Create: `iron-canvas/crates/iron-canvas-core/tests/diagnostics.rs`
- Modify: `iron-canvas/crates/iron-canvas-web/Cargo.toml` (dev-tools → core/dev-diagnostics)

**Interfaces:**
- Consumes: existing `Orchestrator`, `RendererCore`, `MemSurface` (recorder), `TestModel` (tests/common/mod.rs).
- Produces (used by Tasks 2-7, exact names):
  - `pub struct FrameDiagnostics` (in `iron_canvas_core::renderer::diag`, re-exported at crate root) — full shape below; sections fill in across Tasks 2-4.
  - `pub fn Orchestrator::set_frame_diagnostics_enabled(&mut self, enabled: bool)` — cfg `dev-diagnostics`; delegates to grid renderer and clears published state on disable.
  - `pub fn Orchestrator::frame_diagnostics(&self) -> Option<FrameDiagnostics>` — cfg `dev-diagnostics`; clone of last published snapshot.
  - `pub(crate) fn RendererCore::publish_diag(&self, attempt_seq: u64, selected: Option<PaintRegimeTag>, work: WorkFlags, effective: Option<PaintRegimeTag>, committed_seq: Option<u64>, outcome: FrameOutcome, layers: DiagPaintedLayers, resolution: DiagCacheResolution)` — cfg; no-op when disabled; fills attempt fields, reads `committed_after` from live `grid_cache`, moves capture → published.
  - `pub(crate) fn RendererCore::diag_reset_capture(&self)` — cfg; clears the in-flight buffer.

- [ ] **Step 1: Write the failing test** — create `iron-canvas/crates/iron-canvas-core/tests/diagnostics.rs`:

```rust
//! Native integration tests for the dev-diagnostics snapshot.
//! Harness mirrors tests/orchestrator_regimes.rs: MemSurface + TestModel.

mod common;

use std::rc::Rc;

use iron_canvas_core::{CanvasSize, FrameDiagnostics, Orchestrator, PaintResult};
use iron_canvas_recorder::MemSurface;

use common::TestModel;

fn harness() -> (Orchestrator<MemSurface>, Rc<TestModel>) {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    (orch, model)
}

#[test]
fn disabled_by_default_publishes_no_snapshot() {
    let (mut orch, _model) = harness();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn enable_then_disable_round_trips() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().expect("enabled capture publishes");
    assert_eq!(diag.schema_version, 1);
    assert_eq!(diag.attempt_seq, 1);
    assert_eq!(diag.committed_seq, Some(1));
    orch.set_frame_diagnostics_enabled(false);
    assert!(orch.frame_diagnostics().is_none());
}

#[test]
fn capture_hold_still_publishes_and_keeps_cache_state() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_capture_fail(Some(iron_canvas_core::FrameInputFailure::SelectedSheet));
    orch.request_repaint();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.attempt_seq, 2);
    assert_eq!(diag.committed_seq, None);
    assert_eq!(
        diag.outcome,
        iron_canvas_core::FrameOutcome::HeldOnInputFailure(
            iron_canvas_core::FrameInputFailure::SelectedSheet
        )
    );
    assert_eq!(
        diag.cache.committed_before, diag.cache.committed_after,
        "a held attempt never presents changed cache state"
    );
}

#[test]
fn overlay_only_attempt_commits_without_cache_work() {
    // Live recipe from orchestrator_regimes.rs:1303-1324: an in-viewport
    // selection move is a committed Overlay regime with NO grid cache
    // commit. It must not be mislabelled as held.
    let model = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_active(6, 2);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.cache.resolution,
        iron_canvas_core::DiagCacheResolution::Committed,
        "an Overlay regime commits a transaction; it is not held"
    );
    assert!(diag.committed_seq.is_some());
    assert_eq!(diag.cache.planned_action, None);
    assert!(!diag.painted_layers.grid);
    assert!(diag.painted_layers.overlay);
    assert_eq!(diag.cache.committed_before, diag.cache.committed_after);
    assert!(diag.geometry.is_none());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: compile failure — `frame_diagnostics` / `set_frame_diagnostics_enabled` / `FrameDiagnostics` / `DiagCacheResolution` do not exist.

- [ ] **Step 3: Add the feature** — in `iron-canvas/crates/iron-canvas-core/Cargo.toml`, extend `[features]`:

```toml
surface-introspection = []
# Structured live-frame diagnostic capture. Dev-only: enabled by
# iron-canvas-web's `dev-tools` and by this crate's self dev-dependency
# below. Off in production builds — the renderer/orchestrator retain no
# diagnostic state or allocation when it is disabled.
dev-diagnostics = []
```

and extend the self dev-dependency so integration tests compile the lib with it:

```toml
iron-canvas-core = { path = ".", features = ["surface-introspection", "dev-diagnostics"] }
```

- [ ] **Step 4: Create the domain module** — `iron-canvas/crates/iron-canvas-core/src/renderer/diag.rs`, whole file:

```rust
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagBufferTruth {
    Valid,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagFingerprintTruth {
    Exact,
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
```

- [ ] **Step 5: Wire the field into `RendererCore` and add the capture methods** — in `src/renderer/mod.rs`:

Add the module declaration next to the others:

```rust
#[cfg(feature = "dev-diagnostics")]
pub mod diag;
```

Add the field to `pub struct RendererCore<P: Painter>` (after `trace: Cell<FrameTrace>`):

```rust
    /// Dev-only structured capture state. `pub(crate)` so the gated
    /// capture methods in `renderer::diag` can read it; zero-size
    /// contribution to production builds (feature-gated), and all writes
    /// are no-ops while its `enabled` flag is false.
    #[cfg(feature = "dev-diagnostics")]
    pub(crate) diag: diag::DiagState,
```

In `for_layer` (:235), initialize it inside the struct literal (after `trace: …`):

```rust
            #[cfg(feature = "dev-diagnostics")]
            diag: diag::DiagState::default(),
```

The capture methods themselves live at the end of `src/renderer/diag.rs`
(the module already owns every diagnostic type; `RendererCore`'s fields are
`pub`/`pub(crate)`, so the impl reads `self.grid_cache` directly). Append to
`diag.rs`:

```rust
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
```

Extend `diag.rs`'s import block (at the top of the file, after the existing
`use` lines) with:

```rust
use crate::renderer::cache::BufferTruth;
use crate::renderer::cell::fingerprint::FingerprintTruth;
```

(`RendererCore` and `Painter` are referenced by full path in the impl
header, matching the crate's one-use-site convention; all later tasks add
their capture methods to this same impl block in `diag.rs`.)

One small enabling edit remains:

- In `src/renderer/cell/fingerprint.rs`, add an accessor to `FingerprintState` (after `pub(crate) struct FingerprintState`'s fields):

```rust
impl FingerprintState {
    pub(crate) fn truth(&self) -> FingerprintTruth {
        self.truth.get()
    }
}
```

- [ ] **Step 6: Orchestrator setter/getter and publish call** — in `src/orchestrator.rs`:

Next to `last_trace()` (:690), add:

```rust
    /// Enable or disable structured frame diagnostics (dev builds only).
    /// Disabling clears the retained snapshot; `frame_diagnostics()`
    /// returns `None` until an enabled attempt completes.
    #[cfg(feature = "dev-diagnostics")]
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        self.grid.renderer.set_diag_enabled(enabled);
    }

    /// Last completed attempt's structured diagnostics, or `None` when
    /// capture is disabled or no enabled attempt has completed. Dev
    /// builds only.
    #[cfg(feature = "dev-diagnostics")]
    pub fn frame_diagnostics(&self) -> Option<FrameDiagnostics> {
        self.grid.renderer.last_diag()
    }
```

Import at the top of the file (with the other `use` items):

```rust
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::{DiagCacheResolution, DiagPaintedLayers, FrameDiagnostics};
```

In `paint_if_dirty`, right after `self.grid.renderer.reset_trace();` (:1130):

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.grid.renderer.diag_reset_capture();
```

In `finish_attempt` (:1254): the cache-install step (:1297-1299) stays
exactly as it is — no `cache_committed` flag is derived from it, because
cache-work presence is not transaction outcome. Insert the painted-layer
facts BEFORE the common overlay step consumes `overlay_ctx`:

```rust
        // 1. install the attempt-owned cache commit, then publish the frame
        //    whose pixels and cache metadata it describes. Held outcomes
        //    carry no commit and therefore touch neither persistent cache nor
        //    frame state beyond their explicit rollback/preserve update.
        if let Some(cache_commit) = cache_commit {
            self.grid.commit_grid_cache(cache_commit);
        }
        self.install_frame(frame);

        // (dev only) painted-layer facts for the diagnostics snapshot must
        // be captured before the common overlay step consumes overlay_ctx.
        #[cfg(feature = "dev-diagnostics")]
        let grid_painted = painted_layers.is_some_and(|layers| layers.grid);
        #[cfg(feature = "dev-diagnostics")]
        let overlay_painted = painted_layers.is_some()
            && overlay_ctx
                .as_ref()
                .is_some_and(|ctx| matches!(ctx.work, OverlayWork::Paint));
```

Then, after the `self.last_trace = trace;` publication (:1373), before the
final `result` return, add:

```rust
        // 5b. publish the structured diagnostics snapshot. Read after the
        //     cache commit was installed, so `committed_after` reflects the
        //     committed truth. Resolution comes from the transaction
        //     outcome, never from the presence of a grid cache commit: an
        //     Overlay regime commits with `cache_commit: None`.
        #[cfg(feature = "dev-diagnostics")]
        self.grid.renderer.publish_diag(
            self.attempt_seq,
            selected,
            work_flags,
            effective,
            committed_seq,
            frame_outcome,
            DiagPaintedLayers {
                grid: grid_painted,
                overlay: overlay_painted,
            },
            if frame_outcome == FrameOutcome::Painted {
                DiagCacheResolution::Committed
            } else {
                DiagCacheResolution::HeldForRetry
            },
        );
```

- [ ] **Step 7: Re-export from the crate root** — in `src/lib.rs`, after the `pub use orchestrator::…` line:

```rust
#[cfg(feature = "dev-diagnostics")]
pub use renderer::diag::{
    DiagBlit, DiagBlitResultTag, DiagBufferTruth, DiagCache, DiagCacheActionTag,
    DiagCacheResolution, DiagCacheTruth, DiagDeltaKind, DiagFetch, DiagFetchPurpose,
    DiagFetchRequest, DiagFingerprintActionTag, DiagFingerprintTruth, DiagGeometry,
    DiagPaintCounts, DiagPaintedLayers, DiagRepaint, DiagRepaintReason, DiagRevealedStrip,
    DiagSegment, FrameDiagnostics,
};
```

- [ ] **Step 8: Enable the feature from the web crate** — in `iron-canvas/crates/iron-canvas-web/Cargo.toml`:

```toml
dev-tools = [
    "dep:iron-canvas-recorder",
    "dep:serde_json",
    "iron-canvas-core/surface-introspection",
    "iron-canvas-core/dev-diagnostics",
]
```

- [ ] **Step 9: Run the tests**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: all four pass.

- [ ] **Step 10: Feature-off build gate**

Run: `cargo check -p iron-canvas-core --locked` (no features) and `cargo check --target wasm32-unknown-unknown -p iron-canvas-web --locked`
Expected: both compile; no `diag` symbols in the feature-off build (the `#[cfg(feature = "dev-diagnostics")]` gates make this structural, the check proves it).

- [ ] **Step 11: Commit**

```bash
git add iron-canvas/crates/iron-canvas-core iron-canvas/crates/iron-canvas-web/Cargo.toml
git commit -m "feat(core): add dev-diagnostics snapshot scaffold with transaction-truth publish"
```

---

### Task 2: Attempt summary, geometry, rebuild reason, probe attribution

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/diag.rs` (`diag_begin_attempt` with probe + attempt-start cache sample, `diag_geometry` with probe containment)
- Modify: `iron-canvas/crates/iron-canvas-core/src/orchestrator.rs` (`diag_probe` field, `set_frame_diagnostics_probe`, probe latch in `paint_if_dirty`)
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/prepared.rs` (geometry call sites)
- Modify: `iron-canvas/crates/iron-canvas-core/tests/diagnostics.rs` (new tests)

**Interfaces:**
- Consumes: Task 1's `FrameDiagnostics`, `DiagState`, `publish_diag`.
- Produces:
  - `pub fn Orchestrator::set_frame_diagnostics_probe(&mut self, range: RCRange)` — cfg; attempt-scoped latch.
  - `pub(crate) fn RendererCore::diag_begin_attempt(&self, delta: DiagDeltaKind, rebuild_reason: Option<RebuildReason>, probe: Option<RCRange>)` — called by `paint_if_dirty` after `plan_frame`; creates the capture with classification facts, the probe, and the attempt-start committed cache truth.
  - `pub(crate) fn RendererCore::diag_geometry(&self, frame: &Chrome, layout: GridLayout)` — called at the entry of each grid prepare path; records `geometry` and computes `probe_segments`.
  - `FramePlan.rebuild_reason` gets `#[cfg_attr(not(feature = "dev-diagnostics"), allow(dead_code))]` (doc updated).

- [ ] **Step 1: Write the failing tests** — append to `tests/diagnostics.rs`:

```rust
use iron_canvas_core::chrome::PaneRegion;
use iron_canvas_core::{DiagDeltaKind, GridVerdict, RebuildReason, RCRange};

#[test]
fn cold_start_reports_no_committed_frame_reason_and_delta_rebuild() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Rebuild));
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::NoCommittedFrame));
}

#[test]
fn freeze_rebuild_reports_reason_and_exact_segments() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_frozen_rows(3);
    orch.request_repaint();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::Freeze));
    let geo = diag.geometry.expect("grid-visited attempt has geometry");
    assert_eq!(geo.shape.frozen_rows(), 3);
    assert_eq!(geo.shape.frozen_cols(), 1);
    // Canonical TL/TR/BL/BR order, every region populated.
    let regions: Vec<PaneRegion> = geo.segments.iter().map(|s| s.region).collect();
    assert_eq!(
        regions,
        vec![
            PaneRegion::TopLeft,
            PaneRegion::TopRight,
            PaneRegion::BottomLeft,
            PaneRegion::BottomRight
        ]
    );
    // Frozen band: the TL/TR segments span rows 1..=3.
    assert_eq!(geo.segments[0].range.r2, 3);
    assert_eq!(geo.segments[1].range.r2, 3);
    assert_eq!(geo.segments[0].range.c2, 1);
    assert_eq!(geo.segments[2].range.c2, 1);
}

#[test]
fn probe_reports_exact_containing_segment_and_is_consumed() {
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    // Probe the frozen top-left corner: exactly TL contains it.
    orch.set_frame_diagnostics_probe(RCRange { r1: 1, c1: 1, r2: 1, c2: 1 });
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.probe,
        Some(RCRange { r1: 1, c1: 1, r2: 1, c2: 1 })
    );
    assert_eq!(diag.probe_segments, vec![PaneRegion::TopLeft]);

    // The probe is attempt-scoped: the next attempt consumes nothing.
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.probe, None);
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn probe_outside_all_segments_reports_empty_attribution() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    orch.set_frame_diagnostics_probe(RCRange { r1: 999, c1: 999, r2: 999, c2: 999 });
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert!(diag.probe.is_some());
    assert!(diag.probe_segments.is_empty());
}

#[test]
fn overlay_only_attempt_has_no_geometry_and_no_probe_segments() {
    let model = Rc::new(TestModel::synthetic_grid().with_active(5, 2));
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_active(6, 2);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Stable));
    assert!(diag.geometry.is_none());
    assert_eq!(diag.repaint.verdict, None);
    assert!(diag.probe_segments.is_empty());
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: compile failure — `set_frame_diagnostics_probe`, `DiagDeltaKind`, `diag.probe` etc. do not exist yet.

- [ ] **Step 3: Add the capture methods** — in `src/renderer/diag.rs`, inside the same gated impl block the Task 1 methods live in:

```rust
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
```

These methods need two new imports in diag.rs — add next to the Task 1 imports:

```rust
use crate::chrome::Chrome;
use crate::renderer::prepared::FetchedCells;
```

and `diag_geometry` needs `GridLayout::shape()` — it exists (`GridLayout` in `chrome/pane_region.rs` has `shape()`; if it is `pub(super)`, promote it to `pub` in `pane_region.rs` — a read-only accessor, promotion changes no behavior).

- [ ] **Step 4: Orchestrator probe latch and begin call** — in `src/orchestrator.rs`:

Add the field to `pub struct Orchestrator<S>` (after `commit_seq: u64`):

```rust
    /// Host-supplied expected-change address for the next paint attempt
    /// (dev diagnostics only). Latched by `paint_if_dirty` after the
    /// empty-work short circuit and cleared on consumption. Diagnostic
    /// evidence only — never read by classification, planning, or any
    /// prepare/execute path.
    #[cfg(feature = "dev-diagnostics")]
    diag_probe: Option<RCRange>,
```

Initialize in `new()` (after `commit_seq: 0`):

```rust
            #[cfg(feature = "dev-diagnostics")]
            diag_probe: None,
```

Next to `set_frame_diagnostics_enabled`, add:

```rust
    /// Set the diagnostic probe address for the next non-idle paint
    /// attempt. Attempt-scoped: the next attempt latches it and it is
    /// cleared on consumption. Dev builds only.
    #[cfg(feature = "dev-diagnostics")]
    pub fn set_frame_diagnostics_probe(&mut self, range: RCRange) {
        self.diag_probe = Some(range);
    }
```

In `paint_if_dirty`, between classification (:1180-1186) and `plan_frame`, plus the begin call:

```rust
        let delta = Chrome::classify(
            self.last_frame.as_ref(),
            model_dyn,
            &inputs,
            self.decos.selection().active_cell.as_ref(),
        );
        #[cfg(feature = "dev-diagnostics")]
        let diag_delta = DiagDeltaKind::from(&delta);
        let plan = plan_frame(work, delta, inputs.sheet(), inputs.show_selection());
        // Record the classification facts and latch the attempt's probe
        // before dispatch; the renderer fills the rest during
        // prepare/execute. The probe is consumed here so it can never leak
        // into a later attempt's attribution.
        #[cfg(feature = "dev-diagnostics")]
        self.grid.renderer.diag_begin_attempt(
            diag_delta,
            plan.rebuild_reason,
            self.diag_probe.take(),
        );
```

Add the imports:

```rust
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::DiagDeltaKind;
#[cfg(feature = "dev-diagnostics")]
use crate::types::coord::RCRange;
```

- [ ] **Step 5: Geometry call sites + un-dead the field** — in `src/renderer/prepared.rs`:

In `prepare_full_grid` (:294), right after `let layout = frame.grid_layout();`:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, layout);
```

In `prepare_damage_grid` (:350), right after `let layout = frame.grid_layout();`:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, layout);
```

In `prepare_blit_grid` (:407), right after `let candidate = frame.grid_layout();`:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_geometry(frame, candidate);
```

In `src/orchestrator.rs`, `FramePlan.rebuild_reason` (:144-149): replace `#[allow(dead_code)]` with:

```rust
    #[cfg_attr(not(feature = "dev-diagnostics"), allow(dead_code))]
```

and rewrite its doc:

```rust
    /// Which hard break or scroll incompatibility fired, when `grid` is
    /// `Fresh` because of one. Read by the dev-diagnostics capture (the
    /// only reader) after `plan_frame`; unread in feature-off builds.
    rebuild_reason: Option<RebuildReason>,
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: all pass, including the five new ones.

- [ ] **Step 7: Full core suite**

Run: `cargo test -p iron-canvas-core --locked`
Expected: green — no behavior changed; the only edits outside new code are feature-gated.

- [ ] **Step 8: Commit**

```bash
git add iron-canvas/crates/iron-canvas-core
git commit -m "feat(core): capture classification, geometry, and probe attribution"
```

---

### Task 3: Fetch request records + repaint decision reasons (complete grid verdicts)

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/diag.rs` (`diag_fetch`, `diag_repaint`)
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/cell/fingerprint.rs` (`RepaintReason`, `RepaintDecision`, `compare_to_painted`, `plan_grid_repaint`, unit tests)
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/prepared.rs` (`PreparedRepaint.reason: Option<RepaintReason>`; construction sites :337-346; fetch call sites :306/:384/:450; repaint record sites :542/:566/:594)
- Modify: `iron-canvas/crates/iron-canvas-core/tests/diagnostics.rs` (new tests)

**Interfaces:**
- Consumes: Tasks 1-2 types; `trace_fetch` counters stay as-is.
- Produces:
  - `pub(crate) enum RepaintReason { NoPaintedHistory, LayoutMismatch, RowAddressMismatch, SpanCapExceeded, BorderSafety, FingerprintsEqual, ChangedRows }` in `fingerprint.rs`.
  - `pub(crate) struct RepaintDecision { pub plan: RepaintPlan, pub reason: RepaintReason }` — returned by `compare_to_painted` and `plan_grid_repaint`. `GridVerdict::from(&RepaintPlan)` unchanged.
  - `PreparedRepaint { plan, candidate, reason }` where `reason: Option<RepaintReason>`: `Some` only when the fingerprint comparison ran; `None` for Fresh-built geometry (the captured `rebuild_reason` is the authority there).
  - `pub(crate) fn RendererCore::diag_fetch(&self, purpose: DiagFetchPurpose, region: Option<PaneRegion>, range: RCRange)`.
  - `pub(crate) fn RendererCore::diag_repaint(&self, verdict: GridVerdict, reason: Option<RepaintReason>, changed_rows: &[RowSpan])` — the single verdict recorder for all three grid arms, so Damage and Blit also report `Strip` and the structured snapshot never disagrees with the one-line trace.

- [ ] **Step 1: Write the failing tests** — append to `tests/diagnostics.rs`:

```rust
use iron_canvas_core::{DiagFetchPurpose, DiagRepaintReason, RowSpan};

#[test]
fn unchanged_content_skip_reports_fingerprints_equal() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Skip));
    assert_eq!(
        diag.repaint.reason,
        Some(DiagRepaintReason::FingerprintsEqual)
    );
    assert!(diag.repaint.changed_rows.is_empty());
}

#[test]
fn one_changed_row_reports_exact_span_and_reason() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_cell(4, 2, "new value");
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(
        diag.repaint.verdict,
        Some(GridVerdict::Rows { spans: 1, rows: 1 })
    );
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::ChangedRows));
    assert_eq!(diag.repaint.changed_rows, vec![RowSpan { r1: 4, r2: 4 }]);
}

#[test]
fn span_cap_promotes_full_with_reason() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    // Nine disjoint changed rows exceed the fingerprint planner's 8-span cap.
    for (i, row) in [1, 3, 5, 7, 9, 11, 13, 15, 17].iter().enumerate() {
        model.set_cell(*row, 2, &format!("v{i}"));
    }
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::SpanCapExceeded));
}

#[test]
fn border_safety_promotes_full_with_reason() {
    use iron_canvas_core::{
        Border, BorderItem, BorderStyle, CellStyle,
    };
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    // An explicit border on the changed row itself makes the band's edge
    // unsafe to repaint in isolation (fingerprint's border-safety check).
    model.set_style(
        4,
        2,
        CellStyle {
            border: Some(Border {
                top: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: Some("#000000".to_string()),
                }),
                ..Border::default()
            }),
            ..CellStyle::default()
        },
    );
    model.set_cell(4, 2, "bordered");
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(diag.repaint.reason, Some(DiagRepaintReason::BorderSafety));
}

#[test]
fn fresh_rebuild_full_carries_no_fingerprint_reason() {
    // A freeze rebuild has painted history but the comparison never ran:
    // the snapshot must not fabricate `noPaintedHistory`. The captured
    // rebuildReason is the authority instead.
    let mut orch = Orchestrator::<MemSurface>::new(MemSurface::new(), MemSurface::new());
    let model = Rc::new(TestModel::new().with_data_until(40).with_frozen(2, 1));
    orch.set_model(model.clone());
    orch.resize(CanvasSize { w: 800.0, h: 600.0 }, 1.0);
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);

    model.set_frozen_rows(3);
    orch.request_repaint();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Full));
    assert_eq!(diag.repaint.reason, None);
    assert_eq!(diag.rebuild_reason, Some(RebuildReason::Freeze));
}

#[test]
fn damage_strip_reports_strip_verdict_without_reason() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_cell(4, 2, "damage edit");
    orch.mark_rows_damaged(0, 4, 4);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.repaint.verdict, Some(GridVerdict::Strip));
    assert_eq!(diag.repaint.reason, None);
}

#[test]
fn fetch_requests_sum_to_totals_and_match_segments() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.fetch.requests.len(), diag.fetch.batches);
    assert_eq!(
        diag.fetch
            .requests
            .iter()
            .map(|r| r.cells)
            .sum::<usize>(),
        diag.fetch.addressed_cells
    );
    assert_eq!(
        diag.fetch
            .requests
            .iter()
            .map(|r| r.slots)
            .sum::<usize>(),
        diag.fetch.logical_slots
    );
    // The segment cell counts are the renderer's own fetch accounting:
    // their sum equals the addressed-cell total.
    let geo = diag.geometry.unwrap();
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, diag.fetch.addressed_cells);
    // Every request's range lives inside its segment and every request is
    // a full-segment fetch on the cold Fresh frame.
    for request in &diag.fetch.requests {
        assert_eq!(request.purpose, DiagFetchPurpose::FullSegment);
        let region = request.region.unwrap();
        let segment = geo
            .segments
            .iter()
            .find(|s| s.region == region)
            .expect("request region has a segment");
        assert!(request.range.r1 >= segment.range.r1);
        assert!(request.range.r2 <= segment.range.r2);
        assert!(request.range.c1 >= segment.range.c1);
        assert!(request.range.c2 <= segment.range.c2);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: compile errors for the new imports/functions; `DiagFetchPurpose`/`DiagRepaintReason` exist but the snapshot fields are never populated → assertion failures.

- [ ] **Step 3: Refactor the repaint decision to carry a reason** — in `src/renderer/cell/fingerprint.rs`:

Add the reason enum near `RepaintPlan` (:379):

```rust
/// The branch `plan_grid_repaint` / `compare_to_painted` actually took.
/// Recorded at the decision site; never re-derived by diagnostics. Only
/// meaningful when the comparison ran — Fresh-built geometry and
/// Damage/Blit strips never produce one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepaintReason {
    NoPaintedHistory,
    LayoutMismatch,
    RowAddressMismatch,
    SpanCapExceeded,
    BorderSafety,
    FingerprintsEqual,
    ChangedRows,
}

/// One grid-wide repaint decision plus the reason for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepaintDecision {
    pub(crate) plan: RepaintPlan,
    pub(crate) reason: RepaintReason,
}
```

Change `compare_to_painted` (:281-288):

```rust
    pub(crate) fn compare_to_painted(&self, candidate: &GridFingerprint) -> RepaintDecision {
        self.painted
            .borrow()
            .as_ref()
            .map_or_else(
                || RepaintDecision {
                    plan: RepaintPlan::Full,
                    reason: RepaintReason::NoPaintedHistory,
                },
                |painted| plan_grid_repaint(painted, candidate),
            )
    }
```

Change `plan_grid_repaint` (:403-457) to return `RepaintDecision`, tagging every early return — same branches, same plans, only attribution added:

```rust
fn plan_grid_repaint(painted: &GridFingerprint, candidate: &GridFingerprint) -> RepaintDecision {
    if painted.layout != candidate.layout || painted.rows.len() != candidate.rows.len() {
        return RepaintDecision {
            plan: RepaintPlan::Full,
            reason: RepaintReason::LayoutMismatch,
        };
    }

    let mut spans = Vec::<RowSpan>::new();
    for ((painted_row, painted_digest), (candidate_row, candidate_digest)) in
        painted.rows.iter().zip(&candidate.rows)
    {
        if painted_row != candidate_row {
            return RepaintDecision {
                plan: RepaintPlan::Full,
                reason: RepaintReason::RowAddressMismatch,
            };
        }
        if painted_digest == candidate_digest {
            continue;
        }
        if let Some(last) = spans.last_mut()
            && last.r2 + 1 == *candidate_row
        {
            last.r2 = *candidate_row;
        } else if spans.len() < 8 {
            spans.push(RowSpan {
                r1: *candidate_row,
                r2: *candidate_row,
            });
        } else {
            return RepaintDecision {
                plan: RepaintPlan::Full,
                reason: RepaintReason::SpanCapExceeded,
            };
        }
    }
    if spans.is_empty() {
        return RepaintDecision {
            plan: RepaintPlan::Skip,
            reason: RepaintReason::FingerprintsEqual,
        };
    }

    for span in &spans {
        for frozen in [true, false] {
            let Some(band) = band_rows(candidate.layout, frozen) else {
                continue;
            };
            let band_start = *band.start();
            let band_end = *band.end();
            let start = span.r1.max(band_start);
            let end = span.r2.min(band_end);
            if start > end {
                continue;
            }
            if start > band_start && rows_have_border(painted, candidate, [start - 1, start]) {
                return RepaintDecision {
                    plan: RepaintPlan::Full,
                    reason: RepaintReason::BorderSafety,
                };
            }
            if end < band_end && rows_have_border(painted, candidate, [end, end + 1]) {
                return RepaintDecision {
                    plan: RepaintPlan::Full,
                    reason: RepaintReason::BorderSafety,
                };
            }
        }
    }

    RepaintDecision {
        plan: RepaintPlan::Rows(spans),
        reason: RepaintReason::ChangedRows,
    }
}
```

Update the fingerprint unit test `repaint_plans_skip_rows_and_full` (:814-840) to assert plans and reasons together:

```rust
    #[test]
    fn repaint_plans_skip_rows_and_full() {
        let exact_layout = layout(10, 8, 2, 2);
        let painted = build(exact_layout);
        let decision = plan_grid_repaint(&painted, &painted);
        assert_eq!(decision.plan, RepaintPlan::Skip);
        assert_eq!(decision.reason, RepaintReason::FingerprintsEqual);

        let mut changed = painted.clone();
        let row = changed.scroll_band_start + 1;
        changed.rows[row].1.digest ^= 1;
        let decision = plan_grid_repaint(&painted, &changed);
        assert_eq!(
            decision.plan,
            RepaintPlan::Rows(vec![RowSpan {
                r1: changed.rows[row].0,
                r2: changed.rows[row].0,
            }])
        );
        assert_eq!(decision.reason, RepaintReason::ChangedRows);

        let mut unsafe_change = changed.clone();
        unsafe_change.rows[row].1.has_any_explicit_border = true;
        let decision = plan_grid_repaint(&painted, &unsafe_change);
        assert_eq!(decision.plan, RepaintPlan::Full);
        assert_eq!(decision.reason, RepaintReason::BorderSafety);

        let shifted = build(layout(11, 8, 2, 2));
        let decision = plan_grid_repaint(&painted, &shifted);
        assert_eq!(decision.plan, RepaintPlan::Full);
        assert_eq!(decision.reason, RepaintReason::LayoutMismatch);
    }
```

- [ ] **Step 4: Thread the reason through preparation** — in `src/renderer/prepared.rs`:

Extend `PreparedRepaint` (:201-204):

```rust
pub(crate) struct PreparedRepaint {
    pub(crate) plan: RepaintPlan,
    pub(crate) candidate: GridFingerprint,
    /// `Some` only when the fingerprint comparison ran. Fresh-built
    /// geometry repaints `Full` without a comparison — its authority is
    /// the attempt's `RebuildReason`, not a fingerprint branch.
    pub(crate) reason: Option<RepaintReason>,
}
```

Update the construction site in `prepare_full_grid` (:337-346):

```rust
        let (plan, reason) = if frame.kind.reuses_slots() {
            let decision = self.grid_cache.fingerprint.compare_to_painted(&candidate);
            (decision.plan, Some(decision.reason))
        } else {
            (RepaintPlan::Full, None)
        };
        Some(PreparedGrid::Full {
            layout,
            segments,
            repaint: Some(PreparedRepaint {
                plan,
                candidate,
                reason,
            }),
            cache_action: GridCacheAction::Replace { layout },
        })
```

Update the import (:15-17):

```rust
use crate::renderer::cell::fingerprint::{
    GridFingerprint, GridLayoutTransition, RepaintPlan, RepaintReason, RowShiftIneligible,
    StripFingerprintSource,
};
```

- [ ] **Step 5: Record fetch requests and repaint verdicts** — in `src/renderer/diag.rs`, add to the gated impl block:

```rust
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
            RepaintReason::SpanCapExceeded => DiagRepaintReason::SpanCapExceeded,
            RepaintReason::BorderSafety => DiagRepaintReason::BorderSafety,
            RepaintReason::FingerprintsEqual => DiagRepaintReason::FingerprintsEqual,
            RepaintReason::ChangedRows => DiagRepaintReason::ChangedRows,
        });
        capture.repaint.changed_rows = changed_rows.to_vec();
    }
```

Add imports to diag.rs:

```rust
use crate::renderer::cell::fingerprint::{RepaintPlan, RepaintReason};
```

- [ ] **Step 6: Call sites in `prepared.rs`**:

After `self.trace_fetch(range);` in `prepare_full_grid` (:306):

```rust
            #[cfg(feature = "dev-diagnostics")]
            self.diag_fetch(DiagFetchPurpose::FullSegment, Some(region), range);
```

After `self.trace_fetch(strip_range);` in `prepare_damage_grid` (:384):

```rust
                #[cfg(feature = "dev-diagnostics")]
                self.diag_fetch(
                    DiagFetchPurpose::DamageStrip,
                    Some(grid_segment.region()),
                    strip_range,
                );
```

After `self.trace_fetch(range);` in `prepare_blit_grid` (:450):

```rust
            #[cfg(feature = "dev-diagnostics")]
            self.diag_fetch(DiagFetchPurpose::BlitReveal, Some(region), range);
```

In `execute_prepared_grid`:

Full arm, replace the Task-2-era `diag_repaint` call after `self.trace_grid(GridVerdict::from(&repaint.plan));` (:542) with:

```rust
                #[cfg(feature = "dev-diagnostics")]
                {
                    let verdict = GridVerdict::from(&repaint.plan);
                    self.diag_repaint(
                        verdict,
                        repaint.reason,
                        match &repaint.plan {
                            RepaintPlan::Rows(spans) => spans.as_slice(),
                            RepaintPlan::Skip | RepaintPlan::Full => &[],
                        },
                    );
                }
```

Damage arm, after `self.trace_grid(GridVerdict::Strip);` (:566):

```rust
                #[cfg(feature = "dev-diagnostics")]
                self.diag_repaint(GridVerdict::Strip, None, &[]);
```

Blit arm, after `self.trace_grid(GridVerdict::Strip);` (:594):

```rust
                #[cfg(feature = "dev-diagnostics")]
                self.diag_repaint(GridVerdict::Strip, None, &[]);
```

Add the gated import at the top of `prepared.rs`:

```rust
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::DiagFetchPurpose;
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics` then `cargo test -p iron-canvas-core --locked`
Expected: all green. Any other test touching `compare_to_painted`/`plan_grid_repaint`/`PreparedRepaint` will fail to compile and is fixed by the mechanical tuple → struct change above; the full-suite run is the check.

- [ ] **Step 8: Commit**

```bash
git add iron-canvas/crates/iron-canvas-core
git commit -m "feat(core): record fetch requests and repaint verdicts with honest reasons"
```

---

### Task 4: Cache transitions, blit detail with effective clip, paint counts

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/diag.rs` (`diag_cache_planned`, `diag_fingerprint_action`, `diag_blit`, `diag_blit_revealed`, `diag_blit_clip`, `diag_paint_counts`)
- Modify: `iron-canvas/crates/iron-canvas-core/src/renderer/prepared.rs` (call sites; blit delta computation)
- Modify: `iron-canvas/crates/iron-canvas-core/src/orchestrator.rs` (`diag_blit` call for the `FreshFallback` arm in `paint_viewport_regime`)
- Modify: `iron-canvas/crates/iron-canvas-core/tests/diagnostics.rs` (new tests)

**Interfaces:**
- Consumes: Tasks 1-3. `publish_diag` already fills `resolution` + `committed_after` (Task 1) and `diag_begin_attempt` already samples `committed_before` at attempt start (Task 2). This task fills `planned_action`, `fingerprint_action`, `blit`, `paint_counts`.
- Produces:
  - `pub(crate) fn RendererCore::diag_cache_planned(&self, action: DiagCacheActionTag)`.
  - `pub(crate) fn RendererCore::diag_fingerprint_action(&self, action: DiagFingerprintActionTag)`.
  - `pub(crate) fn RendererCore::diag_blit(&self, plan: &BlitPlan, result: DiagBlitResultTag, cold_cache: Option<bool>, previous: Option<GridLayout>, candidate: GridLayout)` — fills axis/src/dst/strip/result/cold_cache and computes the logical `delta`. `clip` is filled separately by `diag_blit_clip` at the `push_clip` site.
  - `pub(crate) fn RendererCore::diag_blit_revealed(&self, region: PaneRegion, range: RCRange)`.
  - `pub(crate) fn RendererCore::diag_blit_clip(&self, clip: PixelRect)` — records the exact rectangle handed to `Painter::push_clip`.
  - `pub(crate) fn RendererCore::diag_paint_counts(&self, rows: usize, cells: usize)`.

- [ ] **Step 1: Write the failing tests** — append to `tests/diagnostics.rs`:

```rust
use iron_canvas_core::geometry::prim::Axis;
use iron_canvas_core::{
    DiagBlitResultTag, DiagBufferTruth, DiagCacheActionTag, DiagCacheResolution,
    DiagFingerprintActionTag, DiagFingerprintTruth, FrameOutcome,
};

#[test]
fn row_blit_reports_shift_revealed_strip_and_effective_clip() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_top_row(5);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.delta, Some(DiagDeltaKind::Scroll));
    let blit = diag.blit.expect("one-axis scroll blits");
    assert_eq!(blit.axis, Axis::Row);
    assert_eq!(blit.delta, 4);
    assert_eq!(blit.result, DiagBlitResultTag::Shifted);
    assert!(blit.cold_cache.is_none());
    // The revealed strip covers exactly the four newly scrolled-in rows.
    let revealed_rows: i32 = blit
        .revealed
        .iter()
        .map(|strip| strip.range.r2 - strip.range.r1 + 1)
        .sum();
    assert_eq!(revealed_rows, 4);
    // The blit's source and destination rectangles differ by the shift.
    assert_ne!(blit.src, blit.dst);
    assert!(blit.strip.width > 0 && blit.strip.height > 0);
    // Today's finalized blit work hands `plan.pixel_strip` to push_clip
    // (blit_work.rs:113), so the effective clip equals the repaint band —
    // the snapshot must record the actual push_clip argument.
    assert_eq!(blit.clip, blit.strip);
    // Fetch requests for a clean shift are reveal-purpose only.
    assert!(diag
        .fetch
        .requests
        .iter()
        .all(|r| r.purpose == DiagFetchPurpose::BlitReveal));
}

#[test]
fn committed_attempt_records_cache_transition() {
    let (mut orch, _model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.cache.resolution, DiagCacheResolution::Committed);
    assert_eq!(diag.cache.planned_action, Some(DiagCacheActionTag::Replace));
    assert_eq!(
        diag.cache.fingerprint_action,
        Some(DiagFingerprintActionTag::Install)
    );
    let before = diag
        .cache
        .committed_before
        .expect("a dispatched attempt samples its starting cache truth");
    assert!(before.layout.is_none());
    assert_eq!(before.buffer_truth, DiagBufferTruth::Stale);
    assert_eq!(before.fingerprint_truth, DiagFingerprintTruth::Stale);
    assert!(diag.cache.committed_after.layout.is_some());
    assert_eq!(diag.cache.committed_after.buffer_truth, DiagBufferTruth::Valid);
    assert_eq!(
        diag.cache.committed_after.fingerprint_truth,
        DiagFingerprintTruth::Exact
    );
    assert!(diag.paint_counts.rows > 0);
    assert!(diag.paint_counts.cells > diag.paint_counts.rows);
}

#[test]
fn held_attempt_keeps_committed_cache_state() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    model.set_bulk_bridge_fail(true);
    orch.mark_content_dirty();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    let diag = orch.frame_diagnostics().unwrap();
    assert_eq!(diag.outcome, FrameOutcome::HeldOnBridgeFailure);
    assert_eq!(diag.cache.resolution, DiagCacheResolution::HeldForRetry);
    assert_eq!(
        diag.cache.committed_before, diag.cache.committed_after,
        "a held attempt must not present candidate cache state as committed"
    );
}

#[test]
fn held_blit_preflight_reports_held_preflight_result() {
    let (mut orch, model) = harness();
    orch.set_frame_diagnostics_enabled(true);
    assert_eq!(orch.paint_if_dirty(), PaintResult::Painted);
    // Fail the revealed-strip fetch: the blit preflight must hold.
    model.set_bulk_bridge_fail_from(Some(1000));
    model.set_top_row(5);
    orch.view_changed();
    assert_eq!(orch.paint_if_dirty(), PaintResult::Retry);
    let diag = orch.frame_diagnostics().unwrap();
    let blit = diag.blit.expect("scroll attempt records blit detail");
    assert_eq!(blit.result, DiagBlitResultTag::HeldPreflight);
}
```

Note on the held-blit test: `set_bulk_bridge_fail_from(1000)` fails only ranges starting at row >= 1000; if the revealed strip's `r1` is below 1000 in the harness geometry, use `set_bulk_bridge_fail(true)` instead and drop the `set_top_row` scroll nuance — the acceptance is: a scroll attempt whose reveal fetch fails reports `HeldPreflight` and the whole attempt holds with `retry`. The assertion set stays; only the failure knob changes.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics`
Expected: compile errors for the new method/type references, then assertion failures once imports resolve.

- [ ] **Step 3: Add the capture methods** — in `src/renderer/diag.rs`, extend the gated impl block:

```rust
    pub(crate) fn diag_cache_planned(&self, action: DiagCacheActionTag) {
        if !self.diag.enabled.get() {
            return;
        }
        let mut slot = self.diag.ensure_capture();
        let capture = slot.as_mut().expect("ensure_capture inserted a frame");
        capture.cache.planned_action = Some(action);
    }

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
            clip: PixelRect::default(),
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
            blit.clip = clip;
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
```

Plus the free helper (in diag.rs, outside the impl):

```rust
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
                if rows {
                    range.r1
                } else {
                    range.c1
                }
            })
    };
    match (origin(previous), origin(candidate)) {
        (Some(before), Some(after)) => after - before,
        _ => 0,
    }
}
```

Add one import to diag.rs (next to the Task 1/2 imports; `BufferTruth`
and `FingerprintTruth` are already imported there):

```rust
use crate::chrome::BlitPlan;
```

- [ ] **Step 4: Call sites in `prepared.rs`**:

In `prepare_full_grid` — record the planned action next to each `cache_action`:

- empty-layout arm (:323-329), after the `Some(PreparedGrid::Full { … })` closes:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_cache_planned(DiagCacheActionTag::Reset);
```

- normal arm (:342-347), after the `Some(PreparedGrid::Full { … })` closes:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_cache_planned(DiagCacheActionTag::Replace);
```

In `prepare_damage_grid`, before its `Some(PreparedGrid::Damage { … })` (:400):

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_cache_planned(DiagCacheActionTag::Splice);
```

In `prepare_blit_grid`:

- the layout-transition fallback (:415-418), before delegating to `prepare_full_grid`:

```rust
        let GridLayoutTransition::Shift { axis } = transition else {
            self.trace_blit_fallback(self.grid_cache.layout().is_none());
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(self.grid_cache.layout().is_none()),
                self.grid_cache.layout(),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        };
```

- the `!same_axis || buffer_truth != Valid` fallback (:423-426), same pattern:

```rust
        if !same_axis || self.grid_cache.buffer_truth() != BufferTruth::Valid {
            self.trace_blit_fallback(self.grid_cache.layout().is_none());
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(self.grid_cache.layout().is_none()),
                self.grid_cache.layout(),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        }
```

- the `finalize_blit_work` failure (:431-438), with `Some(false)` for `cold_cache`:

```rust
        let Some(work) = blit_work::finalize_blit_work(previous, candidate, frame, plan) else {
            // A classified Shift should always expose at least one address
            // strip. If geometry rules drift, repainting the candidate is a
            // safe recovery; treating this as a bridge hold would retry the
            // same impossible plan indefinitely.
            self.trace_blit_fallback(false);
            #[cfg(feature = "dev-diagnostics")]
            self.diag_blit(
                plan,
                DiagBlitResultTag::GridFallback,
                Some(false),
                Some(previous),
                candidate,
            );
            return self.prepare_full_grid(model, frame);
        };
```

- the bridge-failure hold (:451-459), before `return None;`:

```rust
                #[cfg(feature = "dev-diagnostics")]
                self.diag_blit(
                    plan,
                    DiagBlitResultTag::HeldPreflight,
                    None,
                    Some(previous),
                    candidate,
                );
```

- the successful-shift path, when building `PreparedGrid::Blit` (:492-502):

```rust
        #[cfg(feature = "dev-diagnostics")]
        {
            self.diag_blit(
                plan,
                DiagBlitResultTag::Shifted,
                None,
                Some(previous),
                candidate,
            );
            for strip in address_strips.iter().flatten() {
                self.diag_blit_revealed(strip.region, strip.range);
            }
            self.diag_cache_planned(DiagCacheActionTag::Shift);
        }
```

In `execute_prepared_grid`:

Full arm (:542-549), after the `diag_repaint` call from Task 3, record the fingerprint action and paint counts:

```rust
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(DiagFingerprintActionTag::Install);
                    let mut rows = 0usize;
                    let mut cells = 0usize;
                    for grid_segment in layout.segments() {
                        let range = grid_segment.range();
                        let cols = (range.c2 - range.c1 + 1).max(0) as usize;
                        match &repaint.plan {
                            RepaintPlan::Skip => {}
                            RepaintPlan::Full => {
                                rows += (range.r2 - range.r1 + 1).max(0) as usize;
                                cells += FetchedCells::addressed_cells(range);
                            }
                            RepaintPlan::Rows(spans) => {
                                for span in spans {
                                    let r1 = span.r1.max(range.r1);
                                    let r2 = span.r2.min(range.r2);
                                    if r1 <= r2 {
                                        let span_rows = (r2 - r1 + 1) as usize;
                                        rows += span_rows;
                                        cells += span_rows * cols;
                                    }
                                }
                            }
                        }
                    }
                    self.diag_paint_counts(rows, cells);
                }
```

Reset arm (:517-519), before `return GridCacheCommit::Reset;`:

```rust
        #[cfg(feature = "dev-diagnostics")]
        self.diag_fingerprint_action(DiagFingerprintActionTag::Reset);
```

Damage arm (:566-571), after `self.trace_grid(GridVerdict::Strip);`:

```rust
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(DiagFingerprintActionTag::MarkStale);
                    let rows = strips
                        .iter()
                        .map(|strip| (strip.range.r2 - strip.range.r1 + 1) as usize)
                        .sum();
                    let cells = strips
                        .iter()
                        .map(|strip| FetchedCells::addressed_cells(strip.range))
                        .sum();
                    self.diag_paint_counts(rows, cells);
                }
```

Blit arm (:594-601): record the effective clip at the `push_clip` site, then the fingerprint action and paint counts:

```rust
                self.painter.push_clip(pixel_clip);
                #[cfg(feature = "dev-diagnostics")]
                self.diag_blit_clip(pixel_clip);
                for strip in address_strips.iter_mut().flatten() {
                    paint_strip(self, frame, strip.region, strip.range, &mut strip.fetched);
                }
                self.painter.pop_clip();
                self.trace_grid(GridVerdict::Strip);
                #[cfg(feature = "dev-diagnostics")]
                {
                    self.diag_fingerprint_action(match fingerprint {
                        PreparedFingerprintUpdate::Install(_) => {
                            DiagFingerprintActionTag::Install
                        }
                        PreparedFingerprintUpdate::MarkStale => {
                            DiagFingerprintActionTag::MarkStale
                        }
                    });
                    let rows = address_strips
                        .iter()
                        .flatten()
                        .map(|strip| (strip.range.r2 - strip.range.r1 + 1) as usize)
                        .sum();
                    let cells = address_strips
                        .iter()
                        .flatten()
                        .map(|strip| FetchedCells::addressed_cells(strip.range))
                        .sum();
                    self.diag_paint_counts(rows, cells);
                }
```

Add gated imports to `prepared.rs`:

```rust
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::{
    DiagBlitResultTag, DiagCacheActionTag, DiagFingerprintActionTag,
};
```

- [ ] **Step 5: Orchestrator FreshFallback tagging** — in `src/orchestrator.rs`, `paint_viewport_regime` (:1436), in the `Err(prev)` arm:

```rust
            Err(prev) => {
                #[cfg(feature = "dev-diagnostics")]
                self.grid
                    .renderer
                    .diag_blit(&plan, DiagBlitResultTag::FreshFallback, None, None, prev.grid_layout());
                self.paint_fresh_fallback(model, inputs, work, prev)
            }
```

Add the import:

```rust
#[cfg(feature = "dev-diagnostics")]
use crate::renderer::diag::DiagBlitResultTag;
```

Note `diag_blit`'s `previous: None` yields `delta: 0` for the FreshFallback arm (the renderer never saw the committed layout; the trace's `blit_fallback` already names the cause) — acceptable v1 semantics, documented in the struct.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p iron-canvas-core --locked --test diagnostics` then `cargo test -p iron-canvas-core --locked`
Expected: green. Adjust the held-blit failure knob per its Step 1 note if `set_bulk_bridge_fail_from` does not hit the revealed strip (the assertions stay: `HeldPreflight`, attempt holds, prior pixels kept).

- [ ] **Step 7: Commit**

```bash
git add iron-canvas/crates/iron-canvas-core
git commit -m "feat(core): capture cache transitions, blit detail with clip, paint counts"
```

---

### Task 5: Web wire projection + facade methods + native conversion test

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-web/src/wire.rs` (projection + `#[cfg(test)]` conversion test)
- Modify: `iron-canvas/crates/iron-canvas-web/src/orchestrator.rs` (three facade methods)
- Modify: `iron-canvas/crates/iron-canvas-web/tests/render_wasm.rs` (smoke test)

**Interfaces:**
- Consumes: `iron_canvas_core::renderer::diag::*` (re-exported at crate root), Tasks 1-4 snapshot.
- Produces (JS, dev-tools builds only):
  - `IronCanvas.setFrameDiagnosticsEnabled(enabled: bool)`
  - `IronCanvas.setFrameDiagnosticsProbe(r1: i32, c1: i32, r2: i32, c2: i32)` — attempt-scoped latch for the next non-idle paint.
  - `IronCanvas.frameDiagnostics() -> JsValue` — `undefined` during playback or when disabled; otherwise the camelCase snapshot object with `schemaVersion: 1`.

- [ ] **Step 1: Write the failing native conversion test** — in `wire.rs`, at the bottom (inside the cfg):

```rust
#[cfg(all(test, feature = "dev-tools"))]
mod tests {
    use super::*;
    use iron_canvas_core::chrome::{GridLayout, GridShape, PaneRegion};
    use iron_canvas_core::{
        DiagBlitResultTag, DiagBufferTruth, DiagCacheActionTag, DiagCacheResolution,
        DiagCacheTruth, DiagDeltaKind, DiagFingerprintActionTag, DiagFingerprintTruth,
        DiagPaintCounts, DiagPaintedLayers, DiagRepaintReason, FrameDiagnostics, FrameOutcome,
        GridVerdict, RCRange, RebuildReason, RowSpan,
    };

    /// The wire shape is the contract the browser mirrors parse. Prove the
    /// exact field names here, natively, before any browser test relies on
    /// them.
    #[test]
    fn frame_diagnostics_wire_matches_declared_shape() {
        let diag = FrameDiagnostics {
            schema_version: 1,
            attempt_seq: 7,
            committed_seq: Some(6),
            selected: Some(iron_canvas_core::PaintRegimeTag::SlotsReuse),
            effective: Some(iron_canvas_core::PaintRegimeTag::SlotsReuse),
            work: iron_canvas_core::WorkFlags::CONTENT,
            delta: Some(DiagDeltaKind::Stable),
            rebuild_reason: Some(RebuildReason::Freeze),
            outcome: FrameOutcome::Painted,
            painted_layers: DiagPaintedLayers {
                grid: true,
                overlay: false,
            },
            probe: Some(RCRange { r1: 5, c1: 4, r2: 5, c2: 4 }),
            probe_segments: vec![PaneRegion::BottomLeft],
            geometry: None,
            fetch: Default::default(),
            repaint: iron_canvas_core::DiagRepaint {
                verdict: Some(GridVerdict::Rows { spans: 1, rows: 1 }),
                reason: Some(DiagRepaintReason::ChangedRows),
                changed_rows: vec![RowSpan { r1: 5, r2: 5 }],
            },
            cache: iron_canvas_core::DiagCache {
                planned_action: Some(DiagCacheActionTag::Replace),
                fingerprint_action: Some(DiagFingerprintActionTag::Install),
                committed_before: Some(DiagCacheTruth {
                    layout: None,
                    buffer_truth: DiagBufferTruth::Stale,
                    fingerprint_truth: DiagFingerprintTruth::Stale,
                }),
                resolution: DiagCacheResolution::Committed,
                committed_after: DiagCacheTruth {
                    layout: None,
                    buffer_truth: DiagBufferTruth::Valid,
                    fingerprint_truth: DiagFingerprintTruth::Exact,
                },
            },
            blit: None,
            paint_counts: DiagPaintCounts { rows: 1, cells: 21 },
        };

        let wire = FrameDiagnosticsWire::from(&diag);
        let json = serde_json::to_value(&wire).expect("wire serializes");

        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(json["attemptSeq"], 7);
        assert_eq!(json["committedSeq"], 6);
        assert_eq!(json["selected"], "slotsReuse");
        assert_eq!(json["work"], serde_json::json!(["content"]));
        assert_eq!(json["delta"], "stable");
        assert_eq!(json["rebuildReason"], "freeze");
        assert_eq!(json["outcome"]["kind"], "painted");
        assert_eq!(json["paintedLayers"]["grid"], true);
        assert_eq!(json["paintedLayers"]["overlay"], false);
        assert_eq!(
            json["probe"],
            serde_json::json!({ "r1": 5, "c1": 4, "r2": 5, "c2": 4 })
        );
        assert_eq!(json["probeSegments"], serde_json::json!(["bottomLeft"]));
        assert_eq!(json["geometry"], serde_json::Value::Null);
        assert_eq!(json["repaint"]["verdict"]["kind"], "rows");
        assert_eq!(json["repaint"]["verdict"]["spans"], 1);
        assert_eq!(json["repaint"]["reason"], "changedRows");
        assert_eq!(
            json["repaint"]["changedRows"],
            serde_json::json!([{ "r1": 5, "r2": 5 }])
        );
        assert_eq!(json["cache"]["plannedAction"], "replace");
        assert_eq!(json["cache"]["fingerprintAction"], "install");
        assert_eq!(json["cache"]["committedBefore"]["bufferTruth"], "stale");
        assert_eq!(json["cache"]["resolution"], "committed");
        assert_eq!(json["cache"]["committedAfter"]["fingerprintTruth"], "exact");
        assert_eq!(json["blit"], serde_json::Value::Null);
        assert_eq!(json["paintCounts"]["rows"], 1);
        assert_eq!(json["paintCounts"]["cells"], 21);

        let _ = GridLayout::shape; // shape projection exercised by DiagLayoutWire below
        let _ = GridShape::row_lens;
    }
}
```

The last two `let _` lines exist only to silence unused-import lints until
`DiagLayoutWire` appears in this file — remove them when the layout wire is
added (it is, in this same step).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p iron-canvas-web --features dev-tools --locked wire::tests::frame_diagnostics_wire_matches_declared_shape`
Expected: compile failure — `FrameDiagnosticsWire` does not exist.

- [ ] **Step 3: Wire shapes** — append to `src/wire.rs` (all inside `#[cfg(feature = "dev-tools")]`):

```rust
// =============================================================================
// Outbound (Rust->JS, Serialize): dev-only frame diagnostics projection.
// =============================================================================
//
// The engine's `FrameDiagnostics` stays serde-free; this projection is the
// only place that decides wire names and tagging. Versioned by
// `schemaVersion` (DIAG_SCHEMA_VERSION) so a later recorder embedding can
// migrate. Field names are asserted by the native conversion test above.

use iron_canvas_core::renderer::diag::{
    DiagBlit, DiagBlitResultTag, DiagBufferTruth, DiagCache, DiagCacheActionTag,
    DiagCacheResolution, DiagCacheTruth, DiagDeltaKind, DiagFetch, DiagFetchPurpose,
    DiagFetchRequest, DiagFingerprintActionTag, DiagFingerprintTruth, DiagGeometry,
    DiagPaintCounts, DiagPaintedLayers, DiagRepaint, DiagRepaintReason, DiagRevealedStrip,
    DiagSegment, FrameDiagnostics,
};
use iron_canvas_core::{
    FrameInputFailure, GridShape, GridVerdict, PaneRegion, RCRange, RebuildReason, RowSpan,
    WorkFlags,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameDiagnosticsWire {
    pub schema_version: u8,
    pub attempt_seq: u64,
    pub committed_seq: Option<u64>,
    pub selected: Option<PaintRegimeTag>,
    pub effective: Option<PaintRegimeTag>,
    pub work: Vec<&'static str>,
    pub delta: Option<DiagDeltaKindWire>,
    pub rebuild_reason: Option<RebuildReasonWire>,
    pub outcome: FrameOutcomeWire,
    pub painted_layers: DiagPaintedLayersWire,
    pub probe: Option<RCRangeWireOut>,
    pub probe_segments: Vec<PaneRegionWire>,
    pub geometry: Option<DiagGeometryWire>,
    pub fetch: DiagFetchWire,
    pub repaint: DiagRepaintWire,
    pub cache: DiagCacheWire,
    pub blit: Option<DiagBlitWire>,
    pub paint_counts: DiagPaintCountsWire,
}

impl From<&FrameDiagnostics> for FrameDiagnosticsWire {
    fn from(diag: &FrameDiagnostics) -> Self {
        let mut work = Vec::new();
        if diag.work.contains(WorkFlags::VIEW) {
            work.push("view");
        }
        if diag.work.contains(WorkFlags::CONTENT) {
            work.push("content");
        }
        if diag.work.contains(WorkFlags::GEOMETRY) {
            work.push("geometry");
        }
        if diag.work.contains(WorkFlags::OVERLAY) {
            work.push("overlay");
        }
        Self {
            schema_version: diag.schema_version,
            attempt_seq: diag.attempt_seq,
            committed_seq: diag.committed_seq,
            selected: diag.selected,
            effective: diag.effective,
            work,
            delta: diag.delta.map(DiagDeltaKindWire::from),
            rebuild_reason: diag.rebuild_reason.map(RebuildReasonWire::from),
            outcome: FrameOutcomeWire::from(diag.outcome),
            painted_layers: DiagPaintedLayersWire {
                grid: diag.painted_layers.grid,
                overlay: diag.painted_layers.overlay,
            },
            probe: diag.probe.map(RCRangeWireOut::from),
            probe_segments: diag.probe_segments.iter().copied().map(PaneRegionWire::from).collect(),
            geometry: diag.geometry.as_ref().map(DiagGeometryWire::from),
            fetch: DiagFetchWire::from(&diag.fetch),
            repaint: DiagRepaintWire::from(&diag.repaint),
            cache: DiagCacheWire::from(&diag.cache),
            blit: diag.blit.as_ref().map(DiagBlitWire::from),
            paint_counts: DiagPaintCountsWire {
                rows: diag.paint_counts.rows,
                cells: diag.paint_counts.cells,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagDeltaKindWire {
    Stable,
    Scroll,
    Rebuild,
}

impl From<DiagDeltaKind> for DiagDeltaKindWire {
    fn from(kind: DiagDeltaKind) -> Self {
        match kind {
            DiagDeltaKind::Stable => Self::Stable,
            DiagDeltaKind::Scroll => Self::Scroll,
            DiagDeltaKind::Rebuild => Self::Rebuild,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RebuildReasonWire {
    NoCommittedFrame,
    Size,
    Dpr,
    Theme,
    Model,
    Sheet,
    Freeze,
    Headers,
    TwoAxisScroll,
    MissingActiveSnapshot,
    ActiveCellChangedOrUnknown,
    IncompatibleScrollOverlap,
}

impl From<RebuildReason> for RebuildReasonWire {
    fn from(reason: RebuildReason) -> Self {
        match reason {
            RebuildReason::NoCommittedFrame => Self::NoCommittedFrame,
            RebuildReason::Size => Self::Size,
            RebuildReason::Dpr => Self::Dpr,
            RebuildReason::Theme => Self::Theme,
            RebuildReason::Model => Self::Model,
            RebuildReason::Sheet => Self::Sheet,
            RebuildReason::Freeze => Self::Freeze,
            RebuildReason::Headers => Self::Headers,
            RebuildReason::TwoAxisScroll => Self::TwoAxisScroll,
            RebuildReason::MissingActiveSnapshot => Self::MissingActiveSnapshot,
            RebuildReason::ActiveCellChangedOrUnknown => Self::ActiveCellChangedOrUnknown,
            RebuildReason::IncompatibleScrollOverlap => Self::IncompatibleScrollOverlap,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum FrameOutcomeWire {
    Painted,
    HeldOnBridgeFailure,
    HeldOnInputFailure { input: FrameInputFailureWire },
}

impl From<iron_canvas_core::FrameOutcome> for FrameOutcomeWire {
    fn from(outcome: iron_canvas_core::FrameOutcome) -> Self {
        match outcome {
            iron_canvas_core::FrameOutcome::Painted => Self::Painted,
            iron_canvas_core::FrameOutcome::HeldOnBridgeFailure => Self::HeldOnBridgeFailure,
            iron_canvas_core::FrameOutcome::HeldOnInputFailure(input) => {
                Self::HeldOnInputFailure {
                    input: FrameInputFailureWire::from(input),
                }
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FrameInputFailureWire {
    SelectedSheet,
    SelectedView,
    SheetMismatch,
    FrozenRows,
    FrozenColumns,
    RowHeaderVisibility,
    ColumnHeaderVisibility,
}

impl From<FrameInputFailure> for FrameInputFailureWire {
    fn from(failure: FrameInputFailure) -> Self {
        match failure {
            FrameInputFailure::SelectedSheet => Self::SelectedSheet,
            FrameInputFailure::SelectedView => Self::SelectedView,
            FrameInputFailure::SheetMismatch => Self::SheetMismatch,
            FrameInputFailure::FrozenRows => Self::FrozenRows,
            FrameInputFailure::FrozenColumns => Self::FrozenColumns,
            FrameInputFailure::RowHeaderVisibility => Self::RowHeaderVisibility,
            FrameInputFailure::ColumnHeaderVisibility => Self::ColumnHeaderVisibility,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagPaintedLayersWire {
    pub grid: bool,
    pub overlay: bool,
}

impl From<DiagPaintedLayers> for DiagPaintedLayersWire {
    fn from(layers: DiagPaintedLayers) -> Self {
        Self {
            grid: layers.grid,
            overlay: layers.overlay,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RCRangeWireOut {
    pub r1: i32,
    pub c1: i32,
    pub r2: i32,
    pub c2: i32,
}

impl From<RCRange> for RCRangeWireOut {
    fn from(range: RCRange) -> Self {
        Self {
            r1: range.r1,
            c1: range.c1,
            r2: range.r2,
            c2: range.c2,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PaneRegionWire {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl From<PaneRegion> for PaneRegionWire {
    fn from(region: PaneRegion) -> Self {
        match region {
            PaneRegion::TopLeft => Self::TopLeft,
            PaneRegion::TopRight => Self::TopRight,
            PaneRegion::BottomLeft => Self::BottomLeft,
            PaneRegion::BottomRight => Self::BottomRight,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagSegmentWire {
    pub region: PaneRegionWire,
    pub range: RCRangeWireOut,
    pub cells: usize,
}

impl From<&DiagSegment> for DiagSegmentWire {
    fn from(segment: &DiagSegment) -> Self {
        Self {
            region: PaneRegionWire::from(segment.region),
            range: RCRangeWireOut::from(segment.range),
            cells: segment.cells,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GridShapeWire {
    pub row_lens: [usize; 2],
    pub col_lens: [usize; 2],
    pub frozen_rows: i32,
    pub frozen_cols: i32,
}

impl From<GridShape> for GridShapeWire {
    fn from(shape: GridShape) -> Self {
        Self {
            row_lens: shape.row_lens(),
            col_lens: shape.col_lens(),
            frozen_rows: shape.frozen_rows(),
            frozen_cols: shape.frozen_cols(),
        }
    }
}

/// Geometry: the design's example carries frozen counts at the geometry
/// root (topRow/leftColumn/frozenRows/frozenColumns); the shape object
/// repeats them alongside the exact slot lengths.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagGeometryWire {
    pub canvas: CanvasSizeWire,
    pub dpr: f64,
    pub sheet: u32,
    pub top_row: i32,
    pub left_column: i32,
    pub frozen_rows: i32,
    pub frozen_cols: i32,
    pub row_header_thickness: i32,
    pub col_header_thickness: i32,
    pub show_row_headers: bool,
    pub show_col_headers: bool,
    pub shape: GridShapeWire,
    pub segments: Vec<DiagSegmentWire>,
}

impl From<&DiagGeometry> for DiagGeometryWire {
    fn from(geometry: &DiagGeometry) -> Self {
        Self {
            canvas: CanvasSizeWire::from(geometry.canvas),
            dpr: geometry.dpr,
            sheet: geometry.sheet,
            top_row: geometry.top_row,
            left_column: geometry.left_column,
            frozen_rows: geometry.shape.frozen_rows(),
            frozen_cols: geometry.shape.frozen_cols(),
            row_header_thickness: geometry.row_header_thickness,
            col_header_thickness: geometry.col_header_thickness,
            show_row_headers: geometry.show_row_headers,
            show_col_headers: geometry.show_col_headers,
            shape: GridShapeWire::from(geometry.shape),
            segments: geometry.segments.iter().map(DiagSegmentWire::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagFetchPurposeWire {
    FullSegment,
    DamageStrip,
    BlitReveal,
}

impl From<DiagFetchPurpose> for DiagFetchPurposeWire {
    fn from(purpose: DiagFetchPurpose) -> Self {
        match purpose {
            DiagFetchPurpose::FullSegment => Self::FullSegment,
            DiagFetchPurpose::DamageStrip => Self::DamageStrip,
            DiagFetchPurpose::BlitReveal => Self::BlitReveal,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagFetchRequestWire {
    pub purpose: DiagFetchPurposeWire,
    pub region: Option<PaneRegionWire>,
    pub range: RCRangeWireOut,
    pub cells: usize,
    pub slots: usize,
}

impl From<&DiagFetchRequest> for DiagFetchRequestWire {
    fn from(request: &DiagFetchRequest) -> Self {
        Self {
            purpose: DiagFetchPurposeWire::from(request.purpose),
            region: request.region.map(PaneRegionWire::from),
            range: RCRangeWireOut::from(request.range),
            cells: request.cells,
            slots: request.slots,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagFetchWire {
    pub batches: usize,
    pub addressed_cells: usize,
    pub logical_slots: usize,
    pub requests: Vec<DiagFetchRequestWire>,
}

impl From<&DiagFetch> for DiagFetchWire {
    fn from(fetch: &DiagFetch) -> Self {
        Self {
            batches: fetch.batches,
            addressed_cells: fetch.addressed_cells,
            logical_slots: fetch.logical_slots,
            requests: fetch.requests.iter().map(DiagFetchRequestWire::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum GridVerdictWire {
    Skip,
    Rows { spans: u8, rows: u16 },
    Full,
    Strip,
    Held,
}

impl From<GridVerdict> for GridVerdictWire {
    fn from(verdict: GridVerdict) -> Self {
        match verdict {
            GridVerdict::Skip => Self::Skip,
            GridVerdict::Rows { spans, rows } => Self::Rows { spans, rows },
            GridVerdict::Full => Self::Full,
            GridVerdict::Strip => Self::Strip,
            GridVerdict::Held => Self::Held,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagRepaintReasonWire {
    NoPaintedHistory,
    LayoutMismatch,
    RowAddressMismatch,
    SpanCapExceeded,
    BorderSafety,
    FingerprintsEqual,
    ChangedRows,
}

impl From<DiagRepaintReason> for DiagRepaintReasonWire {
    fn from(reason: DiagRepaintReason) -> Self {
        match reason {
            DiagRepaintReason::NoPaintedHistory => Self::NoPaintedHistory,
            DiagRepaintReason::LayoutMismatch => Self::LayoutMismatch,
            DiagRepaintReason::RowAddressMismatch => Self::RowAddressMismatch,
            DiagRepaintReason::SpanCapExceeded => Self::SpanCapExceeded,
            DiagRepaintReason::BorderSafety => Self::BorderSafety,
            DiagRepaintReason::FingerprintsEqual => Self::FingerprintsEqual,
            DiagRepaintReason::ChangedRows => Self::ChangedRows,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RowSpanWire {
    pub r1: i32,
    pub r2: i32,
}

impl From<RowSpan> for RowSpanWire {
    fn from(span: RowSpan) -> Self {
        Self { r1: span.r1, r2: span.r2 }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagRepaintWire {
    pub verdict: Option<GridVerdictWire>,
    pub reason: Option<DiagRepaintReasonWire>,
    pub changed_rows: Vec<RowSpanWire>,
}

impl From<&DiagRepaint> for DiagRepaintWire {
    fn from(repaint: &DiagRepaint) -> Self {
        Self {
            verdict: repaint.verdict.map(GridVerdictWire::from),
            reason: repaint.reason.map(DiagRepaintReasonWire::from),
            changed_rows: repaint.changed_rows.iter().copied().map(RowSpanWire::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagCacheActionTagWire {
    None,
    Replace,
    Splice,
    Shift,
    Reset,
}

impl From<DiagCacheActionTag> for DiagCacheActionTagWire {
    fn from(tag: DiagCacheActionTag) -> Self {
        match tag {
            DiagCacheActionTag::None => Self::None,
            DiagCacheActionTag::Replace => Self::Replace,
            DiagCacheActionTag::Splice => Self::Splice,
            DiagCacheActionTag::Shift => Self::Shift,
            DiagCacheActionTag::Reset => Self::Reset,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagFingerprintActionTagWire {
    Install,
    MarkStale,
    Reset,
}

impl From<DiagFingerprintActionTag> for DiagFingerprintActionTagWire {
    fn from(tag: DiagFingerprintActionTag) -> Self {
        match tag {
            DiagFingerprintActionTag::Install => Self::Install,
            DiagFingerprintActionTag::MarkStale => Self::MarkStale,
            DiagFingerprintActionTag::Reset => Self::Reset,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagCacheTruthWire {
    pub layout: Option<DiagLayoutWire>,
    pub buffer_truth: String,
    pub fingerprint_truth: String,
}

impl From<&DiagCacheTruth> for DiagCacheTruthWire {
    fn from(truth: &DiagCacheTruth) -> Self {
        Self {
            layout: truth.layout.map(DiagLayoutWire::from),
            buffer_truth: match truth.buffer_truth {
                DiagBufferTruth::Valid => "valid".to_string(),
                DiagBufferTruth::Stale => "stale".to_string(),
            },
            fingerprint_truth: match truth.fingerprint_truth {
                DiagFingerprintTruth::Exact => "exact".to_string(),
                DiagFingerprintTruth::Stale => "stale".to_string(),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagLayoutWire {
    pub shape: GridShapeWire,
    pub segments: Vec<DiagSegmentWire>,
}

impl From<iron_canvas_core::chrome::GridLayout> for DiagLayoutWire {
    fn from(layout: iron_canvas_core::chrome::GridLayout) -> Self {
        Self {
            shape: GridShapeWire::from(layout.shape()),
            segments: layout
                .segments()
                .map(|segment| DiagSegmentWire {
                    region: PaneRegionWire::from(segment.region()),
                    range: RCRangeWireOut::from(segment.range()),
                    cells: (segment.range().r2 - segment.range().r1 + 1).max(0) as usize
                        * (segment.range().c2 - segment.range().c1 + 1).max(0) as usize,
                })
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagCacheResolutionWire {
    Committed,
    HeldForRetry,
}

impl From<DiagCacheResolution> for DiagCacheResolutionWire {
    fn from(resolution: DiagCacheResolution) -> Self {
        match resolution {
            DiagCacheResolution::Committed => Self::Committed,
            DiagCacheResolution::HeldForRetry => Self::HeldForRetry,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagCacheWire {
    pub planned_action: Option<DiagCacheActionTagWire>,
    pub fingerprint_action: Option<DiagFingerprintActionTagWire>,
    pub committed_before: Option<DiagCacheTruthWire>,
    pub resolution: DiagCacheResolutionWire,
    pub committed_after: DiagCacheTruthWire,
}

impl From<&DiagCache> for DiagCacheWire {
    fn from(cache: &DiagCache) -> Self {
        Self {
            planned_action: cache.planned_action.map(DiagCacheActionTagWire::from),
            fingerprint_action: cache.fingerprint_action.map(DiagFingerprintActionTagWire::from),
            committed_before: cache.committed_before.as_ref().map(DiagCacheTruthWire::from),
            resolution: DiagCacheResolutionWire::from(cache.resolution),
            committed_after: DiagCacheTruthWire::from(&cache.committed_after),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AxisWire {
    Row,
    Column,
}

impl From<iron_canvas_core::geometry::prim::Axis> for AxisWire {
    fn from(axis: iron_canvas_core::geometry::prim::Axis) -> Self {
        match axis {
            iron_canvas_core::geometry::prim::Axis::Row => Self::Row,
            iron_canvas_core::geometry::prim::Axis::Column => Self::Column,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagBlitResultTagWire {
    Shifted,
    HeldPreflight,
    GridFallback,
    FreshFallback,
}

impl From<DiagBlitResultTag> for DiagBlitResultTagWire {
    fn from(tag: DiagBlitResultTag) -> Self {
        match tag {
            DiagBlitResultTag::Shifted => Self::Shifted,
            DiagBlitResultTag::HeldPreflight => Self::HeldPreflight,
            DiagBlitResultTag::GridFallback => Self::GridFallback,
            DiagBlitResultTag::FreshFallback => Self::FreshFallback,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagRevealedStripWire {
    pub region: PaneRegionWire,
    pub range: RCRangeWireOut,
}

impl From<&DiagRevealedStrip> for DiagRevealedStripWire {
    fn from(strip: &DiagRevealedStrip) -> Self {
        Self {
            region: PaneRegionWire::from(strip.region),
            range: RCRangeWireOut::from(strip.range),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagBlitWire {
    pub axis: AxisWire,
    pub delta: i32,
    // `PixelRect`/`Point` already derive Serialize in core (they ride the
    // .icr schema) — serialize them directly, as wire.rs already does.
    pub src: iron_canvas_core::geometry::pixel_rect::PixelRect,
    pub dst: iron_canvas_core::geometry::pixel_rect::PixelRect,
    pub clip: iron_canvas_core::geometry::pixel_rect::PixelRect,
    pub strip: iron_canvas_core::geometry::pixel_rect::PixelRect,
    pub revealed: Vec<DiagRevealedStripWire>,
    pub result: DiagBlitResultTagWire,
    pub cold_cache: Option<bool>,
}

impl From<&DiagBlit> for DiagBlitWire {
    fn from(blit: &DiagBlit) -> Self {
        Self {
            axis: AxisWire::from(blit.axis),
            delta: blit.delta,
            src: blit.src,
            dst: blit.dst,
            clip: blit.clip,
            strip: blit.strip,
            revealed: blit.revealed.iter().map(DiagRevealedStripWire::from).collect(),
            result: DiagBlitResultTagWire::from(blit.result),
            cold_cache: blit.cold_cache,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagPaintCountsWire {
    pub rows: usize,
    pub cells: usize,
}

impl From<DiagPaintCounts> for DiagPaintCountsWire {
    fn from(counts: DiagPaintCounts) -> Self {
        Self {
            rows: counts.rows,
            cells: counts.cells,
        }
    }
}
```

Fixups on the way in: `iron_canvas_core::chrome::{GridLayout, GridShape}` must be public paths (they are: `pub use pane_region::{…}` in `chrome/mod.rs`); `GridLayout::shape()` must be `pub` (promoted in Task 2 if needed). `PaintRegimeTag` already derives `Serialize` with snake_case, so it is reused directly.

- [ ] **Step 4: Facade methods** — in `src/orchestrator.rs` (web), after `frame_trace()` (:255-262):

```rust
    /// Enable structured capture for the next `frameDiagnostics()` reads.
    /// Disabled by default; disabling clears the retained snapshot.
    /// Dev-tools builds only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsEnabled")]
    pub fn set_frame_diagnostics_enabled(&mut self, enabled: bool) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_enabled(enabled);
    }

    /// Diagnostic probe address for the next non-idle paint attempt:
    /// the snapshot reports which planned segments contain it. Attempt-
    /// scoped, range-only, never read by the planner. Dev-tools only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "setFrameDiagnosticsProbe")]
    pub fn set_frame_diagnostics_probe(&mut self, r1: i32, c1: i32, r2: i32, c2: i32) {
        self.runtime
            .orchestrator_mut()
            .set_frame_diagnostics_probe(iron_canvas_core::RCRange { r1, c1, r2, c2 });
    }

    /// Structured snapshot of the last completed live attempt.
    /// Returns `undefined` when capture is disabled or during playback;
    /// live callers use `frameTrace()` for the allocation-free one-line
    /// summary. Dev-tools builds only.
    #[cfg(feature = "dev-tools")]
    #[wasm_bindgen(js_name = "frameDiagnostics")]
    pub fn frame_diagnostics(&self) -> JsValue {
        if matches!(self.mode, CanvasMode::Playback(_)) {
            return JsValue::UNDEFINED;
        }
        match self.runtime.orchestrator().frame_diagnostics() {
            None => JsValue::UNDEFINED,
            Some(diag) => {
                let wire = crate::wire::FrameDiagnosticsWire::from(&diag);
                serde_wasm_bindgen::to_value(&wire).unwrap_or(JsValue::UNDEFINED)
            }
        }
    }
```

- [ ] **Step 5: Browser smoke test** — append to `render_wasm.rs`:

```rust
/// Dev-diagnostics wire smoke: enabled capture returns a snapshot object
/// with the attempt fields; disabled capture returns `undefined`.
#[wasm_bindgen_test]
fn stage6_frame_diagnostics_wire_smoke() {
    let store = stage6_fixture_store();
    let top_row = Rc::new(Cell::new(1));
    let left_column = Rc::new(Cell::new(1));
    let (mut canvas, _grid) = stage6_canvas_over(store, top_row, left_column, None);

    assert!(canvas.frame_diagnostics().is_undefined());

    canvas.set_frame_diagnostics_enabled(true);
    // The cold Fresh already ran before this function returned; force a
    // new attempt so an enabled capture actually publishes.
    canvas.mark_content_dirty();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let value = canvas.frame_diagnostics();
    assert!(!value.is_undefined(), "enabled capture must publish");

    let diag: DiagWireMirror = serde_wasm_bindgen::from_value(value).expect("snapshot parses");
    assert_eq!(diag.schema_version, 1);
    assert_eq!(diag.attempt_seq, 2);
    assert!(matches!(diag.outcome, FrameOutcomeMirror::Painted));
    assert_eq!(diag.geometry.as_ref().unwrap().segments.len(), 1);

    canvas.set_frame_diagnostics_enabled(false);
    assert!(canvas.frame_diagnostics().is_undefined());
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagWireMirror {
    schema_version: u8,
    attempt_seq: u64,
    outcome: FrameOutcomeMirror,
    geometry: Option<DiagGeometryMirror>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum FrameOutcomeMirror {
    Painted,
    HeldOnBridgeFailure,
    HeldOnInputFailure { input: String },
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagGeometryMirror {
    segments: Vec<DiagSegmentMirror>,
}

#[derive(serde::Deserialize)]
struct DiagSegmentMirror {
    region: String,
    cells: usize,
}
```

(These mirrors are the smoke's minimal subset; Task 7 defines the full
scenario mirror that the native wire test in Step 1 already pins.)

- [ ] **Step 6: Run the conversion test, smoke test, full browser suite**

Run: `cargo test -p iron-canvas-web --features dev-tools --locked wire::`
Expected: the native wire conversion test passes (host target).

Run: `cargo test --target wasm32-unknown-unknown -p iron-canvas-web --locked stage6_frame_diagnostics_wire_smoke`
Expected: pass.

Run: `cargo test --target wasm32-unknown-unknown -p iron-canvas-web -p iron-canvas-datagrid-web --locked`
Expected: full suite green (regression: nothing existing changed).

- [ ] **Step 7: Prod-absence gate**

Run: `cargo check --target wasm32-unknown-unknown -p iron-canvas-web --locked` (no features)
Expected: compiles; `setFrameDiagnosticsEnabled`/`setFrameDiagnosticsProbe`/`frameDiagnostics` are cfg'd out entirely. Confirm with `wasm-pack build --target web` output size unchanged vs. the pre-task baseline (the snapshot types must not appear in the prod wasm).

- [ ] **Step 8: Commit**

```bash
git add iron-canvas/crates/iron-canvas-web
git commit -m "feat(web): frameDiagnostics wire projection, probe setter, native shape test"
```

---

### Task 6: RustyCalc Perf panel expansion via the one-shot command pattern

**Files:**
- Modify: `src/app_state.rs`
- Modify: `src/perf.rs`
- Modify: `src/components/workbook/worksheet/dev_tools_effects.rs`
- Modify: `src/components/workbook/worksheet/mod.rs`
- Modify: `src/components/workbook/worksheet/raf_loop.rs`
- Modify: `src/components/panels/perf_panel.rs`
- Modify: `styles/panels/perf-panel.css`

**Interfaces:**
- Consumes: `IronCanvas::{set_frame_diagnostics_enabled, frame_diagnostics}` (dev-tools builds only); the established one-shot-command pattern (`RecordingCmd`/`PlaybackCmd` drained by worksheet Effects, `install_playback_effect`'s `poke` parameter); `Popover` (`src/components/ui/popover.rs`).
- Produces: Perf panel toggle + expandable JSON readout + copy action; `PerfTimings.frame_diagnostics` holds the JSON string; `PerfTimings.diag_enabled` mirrors the authoritative canvas state; toggle changes wake the rAF loop and force capture off on panel close/unmount.

- [ ] **Step 1: One-shot command** — in `src/app_state.rs`, next to `RecordingCmd`:

```rust
/// One-shot command from the PerfPanel diagnostics toggle to the
/// Worksheet dispatch Effect: `Some(enabled)` means "set the canvas
/// capture flag". Drains via `set(None)`. Exists in both build flavors —
/// in prod (no `dev-tools`) it is written but never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagCmd {
    Set(bool),
}
```

In `AppState` fields, next to `recording_cmd`:

```rust
    /// Pending diagnostics-toggle command from the PerfPanel. Cleared by
    /// Worksheet once dispatched. See [`DiagCmd`].
    pub diag_cmd: Split<Option<DiagCmd>>,
```

In the `Self { … }` init:

```rust
            diag_cmd: Split::new(None),
```

- [ ] **Step 2: Extend `PerfTimings`** — in `src/perf.rs`, inside `pub struct PerfTimings`:

```rust
    /// Authoritative canvas capture state, mirrored by the worksheet's
    /// diagnostics Effect. The rAF loop reads it (untracked) to decide
    /// whether to sample `frameDiagnostics()`.
    pub diag_enabled: RwSignal<bool>,
    /// JSON string of the last captured `IronCanvas.frameDiagnostics()`.
    /// `None` until capture is enabled and a painted frame completes.
    pub frame_diagnostics: RwSignal<Option<String>>,
```

Initialize in the `Self { … }` literal in `PerfTimings::default()`:

```rust
            diag_enabled: RwSignal::new(false),
            frame_diagnostics: RwSignal::new(None),
```

- [ ] **Step 3: The draining Effect** — in `src/components/workbook/worksheet/dev_tools_effects.rs`:

Add the import:

```rust
use crate::app_state::{AppState, DiagCmd, ExportCmd, PlaybackCmd, RecordingCmd};
```

Append (modeled on `install_playback_effect`, which already takes `poke`):

```rust
/// Diagnostics dispatch — drains `app.diag_cmd` (Set(bool) from the
/// PerfPanel toggle) onto the live `IronCanvas`, mirrors the authoritative
/// state back into `app.perf.diag_enabled`, and pokes the one-shot rAF so
/// a paused loop samples (or stops sampling) the new state on the next
/// frame. Disabling also clears the retained JSON readout.
pub(super) fn install_diag_effect(
    state: WorkbookState,
    app: AppState,
    canvas_handle: CanvasHandle,
    poke: impl Fn() + Clone + 'static,
) {
    Effect::new(move |_| {
        let Some(cmd) = app.diag_cmd.get() else {
            return;
        };
        let DiagCmd::Set(enabled) = cmd;
        canvas_handle.update_value(|slot| {
            let Some(ic) = slot.as_mut() else {
                state
                    .status
                    .set(Some(StatusMessage::Error("canvas not ready".into())));
                return;
            };
            ic.set_frame_diagnostics_enabled(enabled);
        });
        app.perf.diag_enabled.set(enabled);
        if !enabled {
            app.perf.frame_diagnostics.set(None);
        }
        app.diag_cmd.set(None);
        poke();
    });
}
```

- [ ] **Step 4: Install it** — in `src/components/workbook/worksheet/mod.rs`, inside the dev-tools block (:165-169):

```rust
        dev_tools_effects::install_diag_effect(state, app, canvas_handle, poke.clone());
```

(next to the existing three `install_*_effect` calls — `poke` is already in scope there, proven by `install_playback_effect`'s argument).

- [ ] **Step 5: rAF sampling only** — in `src/components/workbook/worksheet/raf_loop.rs`, after the existing `frame_trace` sampling block (:291-299):

```rust
        #[cfg(feature = "dev-tools")]
        if let Some(app) = &app
            && app.perf.diag_enabled.get_untracked()
            && action.publish_trace
        {
            let json = canvas_handle.update_value(|slot| {
                slot.as_mut().and_then(|ic| {
                    let value = ic.frame_diagnostics();
                    if value.is_undefined() {
                        None
                    } else {
                        js_sys::JSON::stringify(&value)
                            .ok()
                            .and_then(|text| text.as_string())
                    }
                })
            });
            if let Some(json) = json {
                app.perf.frame_diagnostics.set(Some(json));
            }
        }
```

No closure-local mutation: the toggle path lives entirely in the Effect
from Step 3, which also owns the wake. This block only reads signals.

- [ ] **Step 6: PerfPanel UI** — in `src/components/panels/perf_panel.rs`:

Add signal plumbing near the other `let … = move ||` lines:

```rust
    #[cfg(feature = "dev-tools")]
    let diag_open = RwSignal::new(false);
    #[cfg(feature = "dev-tools")]
    let diag_pos = RwSignal::new((0, 0));
    #[cfg(feature = "dev-tools")]
    let diag_json = move || app.perf.frame_diagnostics.get();

    #[cfg(feature = "dev-tools")]
    let on_toggle_diag = move |ev: web_sys::MouseEvent| {
        let pos = ev
            .current_target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
            .map(|el| {
                let rect = el.get_bounding_client_rect();
                (rect.left() as i32, rect.top() as i32)
            })
            .unwrap_or((0, 0));
        diag_pos.set(pos);
        let next = !app.perf.diag_enabled.get_untracked();
        diag_open.set(next);
        app.diag_cmd.set(Some(DiagCmd::Set(next)));
    };
    #[cfg(feature = "dev-tools")]
    let on_copy_json = move |_| {
        if let Some(json) = app.perf.frame_diagnostics.get_untracked() {
            // Best-effort clipboard write; surface the synchronous
            // failure (denied permissions, sandboxed iframe) rather than
            // swallowing it. The text stays visible as a manual fallback.
            match window().navigator().clipboard().write_text(&json) {
                Ok(promise) => {
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
                            web_sys::console::warn_1(
                                &format!("[rustycalc diag] clipboard write failed: {e:?}").into(),
                            );
                        }
                    });
                }
                Err(e) => web_sys::console::warn_1(
                    &format!("[rustycalc diag] clipboard unavailable: {e:?}").into(),
                ),
            }
        }
    };

    // Forcing capture off on unmount: closing the Perf panel (or the
    // worksheet) must not leave detailed capture active — it would
    // contaminate later timing samples.
    #[cfg(feature = "dev-tools")]
    on_cleanup(move || app.diag_cmd.set(Some(DiagCmd::Set(false))));
```

In the `view!`, inside the `recording_supported.then(…)` block, after the Record button group:

```rust
            #[cfg(feature = "dev-tools")]
            {
                view! {
                    <span class="pp-sep">"|"</span>
                    <button
                        class="pp-diag-btn"
                        class:active=move || app.perf.diag_enabled.get()
                        title="Capture structured frame diagnostics (frameDiagnostics)"
                        on:click=on_toggle_diag
                        // Stop pointerdown so the Popover's click-outside
                        // does not immediately re-close on the same event.
                        on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
                    >
                        "◉ Diag"
                    </button>
                }.into_any()
            }
```

And after the `frame_trace` span (so the readout renders whenever a JSON
snapshot exists — the Popover itself stays mounted per its own design):

```rust
            #[cfg(feature = "dev-tools")]
            {
                view! {
                    <Popover
                        open=diag_open.read_only()
                        set_open=diag_open.write_only()
                        pos=diag_pos.read_only()
                        above_anchor=true
                        class="pp-diag-popover"
                    >
                        <pre class="pp-diag-json">{move || diag_json().unwrap_or_default()}</pre>
                        <button class="pp-diag-copy" on:click=on_copy_json>"Copy JSON"</button>
                    </Popover>
                }.into_any()
            }
```

Import `Popover` and `DiagCmd`:

```rust
use crate::app_state::{AppState, DiagCmd, ExportCmd, RecordingCmd};
use crate::components::ui::popover::Popover;
```

(verify the `Popover` path against `src/components/ui/mod.rs`; if it is
re-exported there, import from the re-export site instead.)

- [ ] **Step 7: Styles** — append to `styles/panels/perf-panel.css`:

```css
/* Structured diagnostics: button lives in the perf strip; the JSON
 * surface is a fixed-position Popover (viewport-clamped by the Popover
 * component) so the .pp overflow:hidden strip cannot clip it. */

.pp-diag-btn {
    cursor: pointer;
    border: 1px solid var(--border-inner);
    background: transparent;
    color: var(--text-dim);
    border-radius: 3px;
    padding: 0 6px;
}

.pp-diag-btn.active {
    background: var(--accent);
    color: var(--text-primary);
}

.pp-diag-popover {
    position: fixed;
    background: var(--bg-secondary);
    border: 1px solid var(--border-inner);
    border-radius: 4px;
    padding: 6px;
    max-width: 60vw;
    max-height: 50vh;
    overflow: auto;
    z-index: 60;
}

.pp-diag-json {
    margin: 0 0 4px;
    font-family: monospace;
    font-size: 10px;
    white-space: pre;
    color: var(--text-dim);
}

.pp-diag-copy {
    cursor: pointer;
    border: 1px solid var(--border-inner);
    background: transparent;
    color: var(--text-dim);
    border-radius: 3px;
    padding: 1px 6px;
}
```

Substitute the CSS variable names that `perf-panel.css` already uses
(`--bg-secondary`, `--border-inner`, `--text-dim`, `--text-primary` are the
strip's own; `--accent` exists in the app palette — verify against
`styles/` before the first build; the block above matches the file's
existing conventions).

- [ ] **Step 8: Build gates**

Run: `cargo check` and `cargo check --features dev-tools`
Expected: both compile; prod build carries no reference to `frameDiagnostics` (the cfg gates make this structural).

- [ ] **Step 9: Browser smoke (manual, dev build)**

Run: `trunk serve --features dev-tools` (root project). Open the app, open the Perf panel, click `◉ Diag`, commit one cell, confirm: the one-line trace stays visible; the popover shows JSON with `schemaVersion: 1`, a verdict, and a reason; `Copy JSON` puts the text on the clipboard (warn on failure in the console). Click `◉ Diag` again → readout clears. Close the Perf panel while enabled → reopen → toggle shows off (the unmount cleanup forced capture off); `frameDiagnostics()` returns `undefined` from the devtools console.

- [ ] **Step 10: Commit**

```bash
git add src styles
git commit -m "feat(ui): Perf panel diagnostics toggle via one-shot command, Popover JSON view"
```

---

### Task 7: Browser diagnostics scenarios + docs + probe discipline

**Files:**
- Modify: `iron-canvas/crates/iron-canvas-web/tests/render_wasm.rs`
- Modify: `iron-canvas/ARCHITECTURE.md`
- Modify: `iron-canvas/README.md`
- Modify: `iron-canvas/docs/designs/2026-08-16-granular-live-frame-diagnostics.md`
- Create: `iron-canvas/docs/performance/2026-08-16-task4-probe-discipline.md`

**Interfaces:**
- Consumes: Tasks 1-6; live fixture facts: `FixtureStore` = `Rc<RefCell<HashMap<(i32,i32), FixtureCell>>>` seeded `r{row}c{col}` (:1208-1222); `StableViewFixture` owns `frozen_rows`/`frozen_cols`/`top_row`/`left_column` cells (:1224-1253); `stable_canvas_over(store, view)` builds a freeze-capable canvas but paints its cold Fresh before returning (:1366-1382); `stage6_set_value(store, row, col, value)` (:1986); `stage6_scroll_to(canvas, cell, row)` = cell set + `view_changed_js` + paint (:1557-1560); `stage6_assert_matches_forced_fresh` (:2293).

- [ ] **Step 1: Reusable scenario mirror + diagnostic canvas helper** — at the bottom of `render_wasm.rs`:

```rust
// Dev-diagnostics wire mirrors for scenario assertions. Field names are
// pinned by the native wire conversion test in iron-canvas-web/src/wire.rs;
// keep this mirror in exact correspondence with it.

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagScenario {
    schema_version: u8,
    attempt_seq: u64,
    rebuild_reason: Option<String>,
    outcome: FrameOutcomeMirror,
    probe: Option<RcRangeScenario>,
    probe_segments: Vec<String>,
    geometry: Option<DiagGeometryScenario>,
    fetch: DiagFetchScenario,
    repaint: DiagRepaintScenario,
    cache: DiagCacheScenario,
    blit: Option<DiagBlitScenario>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagGeometryScenario {
    top_row: i32,
    left_column: i32,
    frozen_rows: i32,
    frozen_cols: i32,
    segments: Vec<DiagSegmentScenario>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagSegmentScenario {
    region: String,
    range: RcRangeScenario,
    cells: usize,
}

#[derive(serde::Deserialize, Clone, Debug)]
struct RcRangeScenario {
    r1: i32,
    c1: i32,
    r2: i32,
    c2: i32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagFetchScenario {
    batches: usize,
    addressed_cells: usize,
    logical_slots: usize,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagRepaintScenario {
    verdict: Option<VerdictScenario>,
    reason: Option<String>,
    changed_rows: Vec<RowSpanScenario>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum VerdictScenario {
    Skip,
    Rows { spans: u8, rows: u16 },
    Full,
    Strip,
    Held,
}

#[derive(serde::Deserialize)]
struct RowSpanScenario {
    r1: i32,
    r2: i32,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagCacheScenario {
    resolution: String,
    #[serde(rename = "committedBefore")]
    committed_before: Option<TruthScenario>,
    #[serde(rename = "committedAfter")]
    committed_after: TruthScenario,
}

#[derive(serde::Deserialize)]
struct TruthScenario {
    layout: Option<serde_json::Value>,
    buffer_truth: String,
    fingerprint_truth: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagBlitScenario {
    axis: String,
    delta: i32,
    src: RectScenario,
    dst: RectScenario,
    clip: RectScenario,
    strip: RectScenario,
    result: String,
    cold_cache: Option<bool>,
    revealed: Vec<DiagRevealedScenario>,
}

#[derive(serde::Deserialize)]
struct RectScenario {
    width: f64,
    height: f64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiagRevealedScenario {
    region: String,
    range: RcRangeScenario,
}

fn diag_snapshot(canvas: &IronCanvas) -> DiagScenario {
    let value = canvas.frame_diagnostics();
    assert!(
        !value.is_undefined(),
        "frameDiagnostics must publish while enabled"
    );
    serde_wasm_bindgen::from_value(value).expect("snapshot parses")
}

/// `stable_canvas_over` with structured diagnostics enabled BEFORE the
/// cold Fresh paint, so the first published snapshot exists on return.
/// Freeze and scroll controls come from the caller's `StableViewFixture`.
fn stable_diag_canvas_over(
    store: FixtureStore,
    view: StableViewFixture,
) -> (IronCanvas, HtmlCanvasElement, HtmlCanvasElement) {
    let grid = make_canvas();
    let overlay = make_canvas();
    let Ok(mut canvas) = IronCanvas::create(grid.clone(), overlay.clone()) else {
        panic!("create stable-view IronCanvas");
    };
    let Ok(content) = JsBackedModel::try_from_js_value(make_fixture_model(store)) else {
        panic!("stable-view fixture content model passes the duck test");
    };
    canvas.set_model(Rc::new(StableFixtureModel { content, view }));
    canvas.resize(STAGE6_CANVAS_W, STAGE6_CANVAS_H, STAGE6_DPR);
    canvas.set_frame_diagnostics_enabled(true);
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    (canvas, grid, overlay)
}
```

- [ ] **Step 2: Freeze-toggle scenario** (design's B3 observation):

```rust
/// The B3 freeze-toggle observation: a freeze change is a `Fresh` rebuild
/// with `RebuildReason::Freeze`, and the snapshot must attribute the
/// addressed-cell count to exact before/after segments.
#[wasm_bindgen_test]
fn stage6_diag_freeze_toggle_explains_segments() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store, view.clone());

    // Baseline: one unfrozen BottomRight segment.
    let before = diag_snapshot(&canvas);
    assert_eq!(before.geometry.as_ref().unwrap().segments.len(), 1);

    // Activate a 2x1 freeze: geometry work forces Fresh with reason Freeze.
    view.frozen_rows.set(2);
    view.frozen_cols.set(1);
    canvas.request_repaint();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);

    let after = diag_snapshot(&canvas);
    assert_eq!(after.rebuild_reason.as_deref(), Some("freeze"));
    let geo = after.geometry.as_ref().unwrap();
    assert_eq!(geo.frozen_rows, 2);
    assert_eq!(geo.frozen_cols, 1);
    assert_eq!(geo.segments.len(), 4);
    // Every addressed cell the fetch charged is inside exactly one segment.
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, after.fetch.addressed_cells);
    // And the trace line's `fetched=` is exactly 4x the addressed cells.
    assert_eq!(after.fetch.logical_slots, 4 * cells);
    // A rebuild's Full verdict must NOT fabricate a fingerprint reason.
    assert_eq!(after.repaint.reason, None);

    // Deactivate: back to one segment, still Fresh/Freeze.
    view.frozen_rows.set(0);
    view.frozen_cols.set(0);
    canvas.request_repaint();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let off = diag_snapshot(&canvas);
    assert_eq!(off.rebuild_reason.as_deref(), Some("freeze"));
    assert_eq!(off.geometry.as_ref().unwrap().segments.len(), 1);
}
```

- [ ] **Step 3: Isolated per-segment edits, probe-attributed** (design's quadrant observations — every `skip` names its reason, every repaint names its segment):

```rust
/// One edit per real segment, attributed by the probe address: the probe
/// must land in exactly the intended segment, an identical-value edit must
/// `Skip` with `fingerprintsEqual`, and a real change must repaint with
/// `changedRows` intersecting the probe.
#[wasm_bindgen_test]
fn stage6_diag_isolated_edits_attribute_segments_and_skips() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1).with_frozen(2, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store.clone(), view.clone());

    let snapshot = diag_snapshot(&canvas);
    let geo = snapshot.geometry.as_ref().unwrap();
    assert_eq!(geo.segments.len(), 4);
    let cells: usize = geo.segments.iter().map(|s| s.cells).sum();
    assert_eq!(cells, snapshot.fetch.addressed_cells);

    for (region, row, col) in [
        ("topLeft", 1, 1),
        ("topRight", 1, 4),
        ("bottomLeft", 5, 1),
        ("bottomRight", 5, 4),
    ] {
        // Identical-value edit: the fixture seeds `r{row}c{col}`, so
        // writing the same string back must compare equal and skip.
        canvas.set_frame_diagnostics_probe(row, col, row, col);
        stage6_set_value(&store, row, col, &format!("r{row}c{col}"));
        canvas.mark_content_dirty();
        assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
        let diag = diag_snapshot(&canvas);
        assert_eq!(
            diag.probe,
            Some(RcRangeScenario { r1: row, c1: col, r2: row, c2: col })
        );
        assert_eq!(
            diag.probe_segments,
            vec![region.to_string()],
            "the probe must belong to exactly the intended segment"
        );
        assert!(
            matches!(diag.repaint.verdict, Some(VerdictScenario::Skip)),
            "identical-value edit in {region} must skip; got {:?}",
            diag.repaint.verdict
        );
        assert_eq!(
            diag.repaint.reason.as_deref(),
            Some("fingerprintsEqual"),
            "a skip must name its reason"
        );

        // Real value change: repaint must report rows intersecting the
        // probe, and the probe attribution must stay exact.
        canvas.set_frame_diagnostics_probe(row, col, row, col);
        stage6_set_value(&store, row, col, &format!("{region}-changed"));
        canvas.mark_content_dirty();
        assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
        let diag = diag_snapshot(&canvas);
        assert_eq!(diag.probe_segments, vec![region.to_string()]);
        assert!(
            !matches!(diag.repaint.verdict, Some(VerdictScenario::Skip)),
            "a real change in {region} must not skip"
        );
        assert_eq!(diag.repaint.reason.as_deref(), Some("changedRows"));
        assert!(
            diag.repaint
                .changed_rows
                .iter()
                .any(|span| span.r1 <= row && row <= span.r2),
            "changed rows must include the probed row in {region}"
        );
    }
}
```

- [ ] **Step 4: Deep-scroll blit detail** (exact clips + revealed strips, deterministic):

```rust
/// Deep row and column scrolls must expose exact blit geometry: axis,
/// logical delta, effective clip, revealed strips, and a named result.
#[wasm_bindgen_test]
fn stage6_diag_deep_scrolls_expose_blit_clips() {
    let store = stage6_fixture_store();
    let view = StableViewFixture::new(1, 1);
    let (mut canvas, _grid, _overlay) = stable_diag_canvas_over(store, view.clone());

    // Row scroll: origin 1 -> 12, a qualifying single-axis shift.
    stage6_scroll_to(&mut canvas, &view.top_row, 12);
    let row_blit = diag_snapshot(&canvas).blit.expect("row scroll blits");
    assert_eq!(row_blit.axis, "row");
    assert_eq!(row_blit.delta, 11);
    assert_eq!(row_blit.result, "shifted");
    assert!(row_blit.cold_cache.is_none());
    let revealed_rows: i32 = row_blit
        .revealed
        .iter()
        .map(|s| s.range.r2 - s.range.r1 + 1)
        .sum();
    assert_eq!(revealed_rows, 11, "revealed rows equal the logical delta");
    // Effective clip equals the repaint band (finalized blit work hands
    // plan.pixel_strip to push_clip) and is a nonzero band.
    assert_eq!(row_blit.clip.width, row_blit.strip.width);
    assert_eq!(row_blit.clip.height, row_blit.strip.height);
    assert!(row_blit.clip.width > 0.0 && row_blit.clip.height > 0.0);
    assert_ne!(row_blit.src.width, 0.0);

    // Column scroll: origin 1 -> 8.
    view.left_column.set(8);
    canvas.view_changed_js();
    assert_eq!(canvas.paint_if_dirty(), JsPaintResult::Painted);
    let col_blit = diag_snapshot(&canvas).blit.expect("column scroll blits");
    assert_eq!(col_blit.axis, "column");
    assert_eq!(col_blit.delta, 7);
    assert_eq!(col_blit.result, "shifted");
    let revealed_cols: i32 = col_blit
        .revealed
        .iter()
        .map(|s| s.range.c2 - s.range.c1 + 1)
        .sum();
    assert_eq!(revealed_cols, 7, "revealed columns equal the logical delta");
}
```

The deltas are deterministic by construction: `stage6_scroll_to` sets the
origin cell and paints once (`render_wasm.rs:1557-1560`), and the stable
fixture's fixed row height/column width makes a 11-row / 7-column shift a
qualifying blit (the W4 workload exercises the same shape). No
"adjust-to-observed" branches.

- [ ] **Step 5: Retained-pixel regression** — no new code; run the existing retained-path cases and confirm the suite stays green with capture plumbing present:

Run: `cargo test --target wasm32-unknown-unknown -p iron-canvas-web --locked stage6_`
Expected: all `stage6_*` pass, including every `stage6_assert_matches_forced_fresh` case (retained paths still byte-equal forced-Fresh output — diagnostics changed no paint behavior).

- [ ] **Step 6: Docs**:

`iron-canvas/ARCHITECTURE.md` — after the completion-policy section, add:

```markdown
## Live frame diagnostics (dev-tools)

`FrameTrace` stays the allocation-free one-line summary. For structured
drill-down, the `dev-diagnostics` core feature (enabled by
`iron-canvas-web/dev-tools`; off in production builds) captures one typed
`FrameDiagnostics` snapshot per paint attempt:

- the grid `RendererCore` owns a `DiagState` (enable flag, in-flight
  capture, published last snapshot); all writes are no-ops while disabled,
  so disabled capture performs no allocations;
- capture records classification facts (`DiagDeltaKind`, `RebuildReason`),
  the host's attempt-scoped probe address and which planned segments
  contain it, exact `GridLayout` segments, per-request renderer fetch
  attribution (purpose, region, `RCRange`, addressed cells, logical
  slots), the repaint verdict plus the fingerprint branch reason (absent
  for Fresh-built geometry and Damage/Blit strips) and exact changed row
  spans, the prepared cache action / fingerprint action, blit geometry
  (axis, logical delta, src/dst rects, effective push-clip rect, pixel
  strip, revealed strips, result tag), and painted row/cell counts;
- `finish_attempt` is the only publisher, after the cache commit is
  installed. Cache resolution derives from the transaction outcome, not
  cache-work presence: a committed Overlay regime reports `committed`
  with no cache action, and a held attempt always reports
  `committedBefore == committedAfter` with `heldForRetry`;
- the web facade exposes `setFrameDiagnosticsEnabled(bool)`,
  `setFrameDiagnosticsProbe(r1, c1, r2, c2)`, and `frameDiagnostics()`
  (camelCase wire, `schemaVersion: 1`; `undefined` while disabled or
  during playback); RustyCalc's Perf panel toggles capture through the
  one-shot-command pattern, shows the JSON in a Popover, and forces
  capture off when the panel unmounts.

Diagnostics observe the existing capture/plan/prepare/execute/commit
decisions; they never re-run classifiers or alter raster behavior. The
probe is attribution evidence only and never enters planner eligibility.
```

Update the header dates: `last-verified-against` → current `HEAD` short hash; `working-tree-verified` → 2026-08-16.

`iron-canvas/README.md` — in the dev-tools section (:354-365), after the recording description:

```markdown
With `dev-tools`, `IronCanvas` also exports live frame diagnostics:

- `setFrameDiagnosticsEnabled(enabled)` — runtime switch (default off;
  off builds retain no diagnostic state or API);
- `setFrameDiagnosticsProbe(r1, c1, r2, c2)` — attempt-scoped expected-
  change address; the next snapshot reports which segments contain it;
- `frameDiagnostics()` — structured snapshot of the last completed live
  attempt (`schemaVersion: 1`): classification delta and rebuild reason,
  probe attribution, exact grid segments, per-request fetch attribution,
  repaint verdict + reason, cache transition, blit geometry with the
  effective clip, painted row/cell counts. `undefined` while disabled or
  during playback.

Wall time belongs to the host: measure around `paintIfDirty` and keep
detailed capture off during timing runs (see
`docs/performance/2026-08-16-task4-probe-discipline.md`).
```

`iron-canvas/docs/designs/2026-08-16-granular-live-frame-diagnostics.md` — set `**Status:** Implemented (2026-08-16)` and replace the "Questions to settle" section with:

```markdown
## Questions settled at plan time

1. First UI: expandable full-JSON view + copy action in the existing Perf
   panel; one-line trace stays visible. Structured section rendering
   deferred until Task 4 proves which fields matter.
2. Painted row/cell counts are derived at the renderer boundary
   (`execute_prepared_grid`) from prepared ranges — no per-cell overhead.
   Primitive-op counts deferred to a `Painter` counting decorator.
3. Cache truth uses diagnostic-only names (`DiagBufferTruth`,
   `DiagFingerprintTruth`); internal buffer ownership stays private.
4. No recorder schema change; the `.icr` v5 schema is untouched until live
   probes prove the useful fields.

## Review-driven additions

- Segment attribution uses an attempt-scoped, range-only host probe
  (`setFrameDiagnosticsProbe`): the snapshot reports which planned
  segments contain it. It never enters planner eligibility.
- Cache resolution is transaction outcome, not cache-work presence:
  committed Overlay frames report `committed`.
- Fingerprint reasons are absent unless the comparison ran; rebuilds use
  `rebuildReason` as their authority.
- The effective blit clip (`push_clip` argument) is recorded separately
  from the repaint band.
```

`iron-canvas/docs/performance/2026-08-16-task4-probe-discipline.md` — CREATE, content:

```markdown
# Task 4 Probe Discipline

Every Task 4 cost probe records one row with these fields, in this order:

1. **Edit:** exact address (e.g. `B3`) and old/new value class (empty ->
   text, text -> text, formula recalc, style-only).
2. **View state:** top row, left column, frozen rows/cols, DPR, canvas
   CSS size.
3. **One-line trace:** `frameTrace()` of the timed frame.
4. **Snapshot:** `frameDiagnostics()` from a representative run —
   probe attribution, segments, fetch batches/addressed cells/logical
   slots, verdict + reason, painted rows/cells, cache resolution, blit
   detail when applicable.
5. **Host wall time:** `Draw` (Perf panel) or the probe's own
   `performance.now()` bracket around `paintIfDirty`.
6. **Capture flag:** whether `setFrameDiagnosticsEnabled(true)` was active
   during the timed samples. Timing samples must run with capture OFF;
   enable it only for the representative attribution run.

Rules:

- A `grid:skip` run is never reported as the cost of painting a quadrant.
  Only a snapshot proving the intended visible, paint-relevant change
  reached that segment (probe inside exactly that segment, verdict
  `rows`/`FULL` with `changedRows` or a named promotion reason) may be
  costed as a quadrant repaint.
- Never describe a `FULL` promotion without its reason
  (`spanCapExceeded`, `borderSafety`, `layoutMismatch`,
  `rowAddressMismatch`); a rebuild `FULL` has no fingerprint reason —
  quote its `rebuildReason` instead.
- Retained-pixel scenarios still gate on raw Canvas2D `ImageData`
  equality against independent forced-Fresh output
  (`stage6_assert_matches_forced_fresh`); the snapshot explains, the
  raster comparison proves.

Example row (B3 freeze toggle, illustrative numbers):

| Edit | View | Trace | Segments | Fetch | Verdict | Wall | Capture |
| --- | --- | --- | --- | --- | --- | --- | --- |
| freeze on @ B3 | 1,1 2x1 2.0 1600x900 | `Fresh[GEOMETRY\|OVERLAY] grid:FULL fetched=1856` | 4 segs, 464 cells | 4 batches / 464 / 1856 | FULL, reason null, `rebuildReason: freeze` | 3.1 ms | off |
```

- [ ] **Step 7: Final verification gates**

```bash
cargo test -p iron-canvas-core --locked
cargo test --workspace --locked
cargo test -p iron-canvas-web --features dev-tools --locked wire::
cargo check --target wasm32-unknown-unknown --locked
cargo test --target wasm32-unknown-unknown -p iron-canvas-web -p iron-canvas-datagrid-web --locked
cargo check                              # RustyCalc prod flavor
cargo check --features dev-tools         # RustyCalc dev flavor
```

All must pass. Then one representative Task 4 probe per the new discipline doc: capture OFF, run an existing `stage6_*` workload (e.g. W5/W6), record the row; then capture ON for one frame, record the snapshot.

- [ ] **Step 8: Commit**

```bash
git add iron-canvas/crates/iron-canvas-web/tests/render_wasm.rs iron-canvas/ARCHITECTURE.md iron-canvas/README.md iron-canvas/docs
git commit -m "test(web): probe-attributed diagnostics scenarios; docs + probe discipline"
```

---

## Self-Review Notes (plan author, recorded for the implementer)

Spec coverage: design sections map to tasks — feature boundary & runtime boundary → Task 1/5; attempt summary + geometry → Task 2; probe-based segment attribution (review Finding 1) → Task 2 capture + Task 7 scenarios; fetch accounting → Task 3; repaint decision + reasons (comparison-only, review Finding 5) → Task 3; complete grid verdicts incl. `Strip` (review Finding 6) → Task 3; cache transition with transaction-truth resolution (review Finding 2) → Task 1 publish + Task 4 instrumentation; blit detail incl. effective `push_clip` (review Finding 6) → Task 4; paint counts → Task 4; wall time stays host-side → Task 7 probe doc; Perf panel via one-shot-command lifecycle (review Finding 4) → Task 6; byte-for-byte comparison stays an existing automated gate → Task 7 Step 5; non-goals honored (no `FrameTrace` change, no recorder change, no values/hashes on the wire, no behavior changes).

Review finding disposition: 1 (probe path) → Tasks 2/7; 2 (resolution from `frame_outcome`, painted facts computed before `overlay_ctx` consumption, dedicated overlay test) → Task 1; 3 (stable fixture + `stable_diag_canvas_over` enabling capture before the cold Fresh, corrected mirrors with frozen counts at geometry root, tagged outcome mirror, `r{row}c{col}` identical-value stimulus, deterministic deltas) → Tasks 5/7; 4 (`DiagCmd` one-shot drained by `install_diag_effect` with `poke`, no closure mutation, `on_cleanup` force-off, Popover fixed-position surface, non-Option clipboard with surfaced failure) → Task 6; 5 (`Option<RepaintReason>`, comparison-only) → Task 3; 6 (`Strip` verdicts in Damage/Blit arms; `DiagBlit.clip` recorded at `push_clip`) → Tasks 3/4; 7 (fetch-total assertion moved from Task 2's freeze test into Task 3's fetch test; no task requires future-task facts) → Tasks 2/3; 8 (Leptos 0.8, `RCRange` arithmetic inline instead of a `cols()` accessor) → tech-stack header and Task 4 code.

Type consistency: `FrameDiagnostics` fields are written by exactly the `diag_*` methods named in each task; the wire mirrors (`DiagGeometryWire` et al.) project the same names and are pinned by the native `wire::tests::frame_diagnostics_wire_matches_declared_shape` test before any browser scenario consumes them; `publish_diag`'s parameter list matches its single call site in `finish_attempt`, whose `grid_painted`/`overlay_painted` facts are computed before the overlay step consumes `overlay_ctx`.

Remaining implementation-time contingencies (named, bounded, no shipped assertions at stake): `GridLayout::shape()` visibility promotion if it is `pub(super)` (Task 2); the held-blit test's failure knob (`set_bulk_bridge_fail_from` vs `set_bulk_bridge_fail`) depending on whether the revealed strip's rows are >= 1000 in the 800x600 harness (Task 4, assertions fixed); the exact `Popover` import path (`crate::components::ui::popover` vs a re-export, Task 6); CSS variable names already in `perf-panel.css` (Task 6).
