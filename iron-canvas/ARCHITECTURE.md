<!-- last-verified-against: c52b63e (2026-08-16) -->
<!-- working-tree-verified: 2026-08-16 -->
<!-- covers: iron-canvas/crates/ iron-canvas/Cargo.toml .github/workflows/test.yml -->

# iron-canvas architecture

`iron-canvas` is the read-only rendering workspace used by RustyCalc. It
paints worksheet data supplied through `CanvasModel`, but it never mutates
that model. The live browser facade renders into two stacked canvases:

- the opaque grid layer owns persistent cell, border, header, and frozen-pane
  pixels;
- the transparent overlay layer owns selection, autofill, clipboard,
  point-mode, formula-reference, hover, and consumer decoration pixels.

Both painting and queries use the same committed `Chrome`, so hit testing
cannot observe candidate geometry that failed to paint.

## Workspace topology

| Crate | Responsibility |
| --- | --- |
| `iron-canvas-core` | Backend-neutral model traits, geometry, frame input capture and classification, planning, transactional paint orchestration, grid segment rendering, queries, decorations, and `Painter`/`Surface` traits. No `web-sys`, `wasm-bindgen`, or IronCalc types. |
| `iron-canvas-canvas2d` | Browser Canvas2D backend: `CanvasPainter`, `WebSurface`, paired `Canvas2dRuntime`, CSS theme bridge, setter caches, DPR handling, and text-measurement memo. No IronCalc dependency. |
| `iron-canvas-web` | Spreadsheet `#[wasm_bindgen]` facade, JS-backed model bridge, recording/playback integration, and SVG/PDF export entry points. |
| `iron-canvas-datagrid` | Pure-Rust standalone table model implementing `CanvasModel` and `CellContentQuery`. |
| `iron-canvas-datagrid-web` | `#[wasm_bindgen]` facade for the standalone data grid, including its hover decoration. |
| `iron-canvas-ironcalc` | `IronCalcModel<'a>` adapter plus the shared IronCalc-to-core style/color conversion and conditional-formatting bridge. |
| `iron-canvas-export` | Pure-`std` SVG and PDF painters/surfaces. Export uses throwaway orchestrators and discards overlay output. |
| `iron-canvas-recorder` | `RecorderPainter`/`MemSurface` test backend and optional `RecordingPainter<P>`/`RecordingSurface<S>` `.icr` capture decorator. |

Deleting every adapter crate must still leave `iron-canvas-core` compiling and
testing on its own. Concrete backends and model adapters belong beside core,
not inside it.

## Ownership map

| State or decision | Owner | Invariant |
| --- | --- | --- |
| queued paint intent | `Orchestrator::pending: PendingWork` | One mergeable value, taken once per paint attempt with `mem::take`. Layers own no dirty gates. |
| scalar attempt inputs | `FrameInputs` | Sheet, view, freeze counts, header visibility, DPR, theme, model generation, and selection visibility are captured once. |
| committed geometry | `Orchestrator::last_frame: Option<Chrome>` | Queries see only committed geometry. |
| geometry verdict | `Chrome::classify` | Exactly one `FrameDelta::{Stable, Scroll, Rebuild}` per attempt. |
| paint plan | `FramePlan` | Grid scope and overlay policy are explicit values; no paint scope is stored on `Chrome`. |
| prepared model data | `FetchedCells` and `PreparedGrid` | Fallible reads may use scratch, but cannot mutate committed `GridCache` buffers or the painted fingerprint tree. |
| grid cache installation | `GridCacheCommit` | Execution returns owned commits; only the completion boundary installs them. |
| frame publication, presentation, retry | `Orchestrator::finish_attempt` | Cache commit is installed before its matching frame is published or surfaces are presented. |
| backend state | `Surface` and `Painter` | Renderer code stays independent of Canvas2D and IronCalc. |

## Paint-attempt lifecycle

Every non-idle `Orchestrator::paint_if_dirty` call follows this pipeline:

```text
PendingWork
    │ mem::take
    ▼
FrameInputs::capture
    │
    ├─ failure ───────────────► Held + retry all taken work
    ▼
Chrome::classify
    │ FrameDelta
    ▼
plan_frame
    │ FramePlan { GridWork, OverlayWork, consumes }
    ▼
prepare ──► execute ──► PaintOutcome
                           │
                           ▼
                 Orchestrator::finish_attempt
                 cache → frame → overlay → present → retry → trace
```

The stages are deliberately separate. A bridge read can fail while preparing,
but it cannot leave half-installed cache metadata, a candidate `Chrome`, or
shifted pixels behind.

### Pending work

`PendingWork` is the single queued-work algebra. It contains:

- `GeometryWork::{Clean, Rebuild}`;
- a `view` flag;
- `ContentWork::{Clean, Rows { sheet, spans }, All}`;
- an `overlay` flag.

Row spans are normalized and adjacent/overlapping spans are merged. More than
eight disjoint spans or rows from different sheets degrades to `All`. Once
work becomes whole-grid, row precision is not recovered.

`WorkFlags::{VIEW, CONTENT, GEOMETRY, OVERLAY}` is only a diagnostic projection
for traces and `.icr` frames. It is not queued state and must not be used to
reconstruct dispatch policy.

### Capture and classification

`FrameInputs::capture` reads these model values in fixed order:

1. selected sheet;
2. selected view, checked to reference the same sheet;
3. frozen row and column counts;
4. row and column header visibility;
5. selection visibility.

Canvas size, DPR, theme, and `model_generation` come from the orchestrator.
Every model scalar except selection visibility is fallible. A failure produces
`FrameOutcome::HeldOnInputFailure`, paints and presents nothing, and requeues
the complete taken `PendingWork`. No synthetic sheet `0`, A1 view, or visible
header default is substituted.

`Chrome::classify(prev, model, inputs, active_cell)` compares the capture with
the committed frame in stable order. It returns:

- `FrameDelta::Stable` when size, DPR, theme, model generation, sheet, freeze,
  header visibility, and effective scroll origins still match;
- `FrameDelta::Scroll(BlitPlan)` for one-axis movement with compatible retained
  overlap and a live active-cell value matching its committed snapshot;
- `FrameDelta::Rebuild(RebuildReason)` for a missing frame, any hard break,
  two-axis movement, unknown/changed active-cell content, or incompatible
  overlap.

This single classifier replaces the former split `is_still_valid` and
`screen_for_blit` verdicts.

### Planning and five strategies

`plan_frame` combines the typed `PendingWork` and `FrameDelta` into a closed
`FramePlan`:

| Selected strategy | `GridWork` | Eligibility and work |
| --- | --- | --- |
| `Overlay` | `None` | Stable, content-free, geometry-free view/overlay work. Reuse committed `Chrome`; repaint only overlay. |
| `Viewport` | `Blit(plan)` | Content-free, geometry-free safe one-axis scroll. Preflight all revealed address strips before moving a pixel. |
| `Damage` | `Rows { sheet, spans }` | Stable geometry and row-addressed content on the visible sheet. Repaint full-width row bands. |
| `SlotsReuse` | `AllContent` | Stable geometry and whole-grid content. Reuse slot vectors, then fingerprint the candidate grid. |
| `Fresh` | `Fresh` | First frame, geometry work, any rebuild delta, or content combined with a real scroll. Rebuild geometry and repaint atomically. |

The planner probes a safe scroll before the stable overlay fallback, because a
host view mark may represent real pixel movement. Changed content is never
blitted. `OverlayWork::{Preserve, Paint}` is computed once in the plan; Fresh
and Viewport always paint it, while Damage/SlotsReuse also paint it when
selection is visible or overlay work was explicitly marked.

`PaintRegimeTag` preserves the five public/recorded strategy names. It is a
data-free attribution tag, not the dispatch value; `GridWork` carries the
payloads and drives the exhaustive match.

## Transactional preparation, execution, and completion
### Prepared grid data

`FetchedCells` bundles the equal-length row-major channels:

- `Vec<Fetched<CellStyle>>`;
- `Vec<Fetched<String>>`;
- `Vec<Fetched<CellKind>>`;
- `Vec<Fetched<CellDecoration>>`.

Canvas2D lifecycle is owned by `Canvas2dRuntime<S>` in the Canvas2D backend.
It retains both HTML canvas handles, the raw painter handles, and the live DPR,
while exposing explicit access to the core `Orchestrator<S>`. Recording wrappers,
models, and application schedulers remain host-owned.

`Fetched<T>` distinguishes `Value`, `Absent`, and `BridgeFailed` across both
single-cell and bulk paths. Decorations use the same type as the other three
channels; `BridgeFailed` must never be collapsed into a blank cell before the
hold decision.
Preparation produces `PreparedGrid::{Full, Damage, Blit}` plus a deferred
`GridCacheAction::{Replace, Shift, Splice, Reset}`. It may reuse renderer
scratch and increment trace counters, but it may not write committed
`SegmentBuffers` content or the painted fingerprint tree.

Execution paints only prepared data and returns one owned `GridCacheCommit`
installed only by `finish_attempt`; a held attempt contributes no entry.

### Completion policy

Every regime returns one private `PaintOutcome`:

| Outcome | Meaning |
| --- | --- |
| `Committed` | Every targeted scope executed. Install its cache commit and publish/present the matching frame. |
| `Held` | Nothing executed or presented. Preserve/roll back the committed frame and retry the required scope widened to the whole grid. |

`finish_attempt` is the only completion boundary. Its order is load-bearing:

1. install `GridCacheCommit`;
2. preserve or replace `last_frame`;
3. refresh committed selection/active-cell overlay state;
4. paint the overlay when the plan requires it;
5. present the grid and overlay layers that painted;
6. merge retry work back into `self.pending`;
7. publish regime, effective regime, work flags, and `FrameTrace`.

Fresh and Viewport are whole-frame atomic. Fresh builds from an orchestrator
`spare_slots` pool without consuming `last_frame`; a failed candidate returns
its slot vectors to the pool. Viewport uses `PreparedBlitFrame`, whose
move-based rollback reconstructs the previous `Chrome` if strip preflight
fails after candidate geometry was built. SlotsReuse and Damage require stable
geometry, so they may safely publish the reused candidate; a bridge failure
holds the whole attempt, prior pixels stay, and content is retried grid-wide.

`PaintResult::Retry` means the scheduler should call `paint_if_dirty` again
without requiring a new host signal.

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

## Chrome and pane geometry

`Chrome` is committed frame geometry, not a bag of paint work. It owns:

- sheet and `PaneSet`;
- row/column header thickness and shared `cell_origin`;
- logical `canvas_size`, DPR, and `Rc<CanvasTheme>`;
- `model_generation` and captured header-visibility flags;
- `FrameKindTag::{Fresh, SlotsReused, Blitted}`.

It does not own row spans, segment buffers, or fingerprints; it derives the
piecewise `GridLayout` of 1–4 dense address segments (TL, TR, BL, BR).
`GridWork` passes paint scope explicitly, preventing a SlotsReuse frame from
inheriting a previous blit's scope.

### Construction paths

`Chrome::next(prev, model, inputs, FramePath)` has two variants:

- `Fresh` walks the model and builds new geometry. `Orchestrator` normally
  calls `Chrome::build` directly with its standing `RecycledSlots` pool so a
  candidate can be held without dismantling the committed frame.
- `SlotsReuse` keeps the previous slot vectors and header labels verbatim,
  refreshing theme, DPR, model generation, header flags, and kind. Stable edit
  repaints therefore skip row/column geometry walks.

`Chrome::prepare_blit` builds a reversible scroll candidate for the live
orchestrator. `Chrome::next_blit` is the immediate-commit convenience wrapper
and returns `BlitOutcome::{Blitted, FreshFallback}`. In-place reuse can still
reject after classification, for example when row-header width changes at a
digit boundary; that path rebuilds Fresh.

### Fresh build phases

`FramePath::Fresh` runs five ordered phases:

```text
A  captured freeze counts
B  row walk into PaneSet
C  measure row-header thickness from the last visible row label
D  column walk using the measured X origin
E  resolve header labels and assemble Chrome
```

The scalar inputs come from `FrameInputs`; only row heights, column widths,
model bounds, and header labels are read during the geometry walk. Hidden
headers have thickness and inset `0`, allowing cells to reclaim that edge.

`PaneSet` owns `AxisSlots<RowSlot>` and `AxisSlots<ColSlot>`. Each axis owns
frozen and scroll vectors, a `frozen_offset`, and `last_id`. Slot coordinates
are absolute integer CSS pixels; DPR remains a separate `f64` backend concern.
The 3-pixel `FROZEN_SEP` gap is woven into the slot vectors by `PaneSet` and
painted after cells so the separator wins shared pixels.

Use `Chrome::hit_test`, `cell_rect`, `range_rect`, `resize_handle_at`,
`scroll_pane_rect`, and `scroll_to_show`; hosts must not duplicate slot or
frozen-pane arithmetic.

## Grid renderer and retained pixels

Fresh, SlotsReuse, Damage, and Viewport share one grid shell:

```text
optional blit shifts
Grid
  Cells          strategy-specific prepared grid work
  FrozenSep      after cells
  Headers        both axes, or only the scrolled axis for Viewport
  Corner         only when both header strips are visible
```

Each painted segment uses five ordered passes over resolved `CellPaint` slots:

1. backgrounds;
2. conditional-format decorations;
3. grid borders;
4. explicit borders;
5. text.

This order keeps decorations below borders, explicit borders above grid
borders on shared edges, and text above neighbouring backgrounds.

### Cache lifetimes

| Lifetime | Type | Purpose |
| --- | --- | --- |
| per-call content, retained capacity | `FrameCache` | `CellPaint` slots, grid-line flag, text lines/wrap string, and a pool of strip `FetchedCells`. Contents are overwritten; capacity survives. |
| cross-frame model/pixel truth | `GridCache` | `SegmentBuffers` per region plus one `FingerprintState`, identity keyed by `GridLayout`. Only installed commits change it. |
| renderer lifetime | `FontIntern`, `ColorIntern`, Canvas2D setter/measurement caches | Avoid repeated allocation and backend crossings. |

The fingerprint is one grid tree keyed by the exact `GridLayout`: one digest
per absolute row folding every dense column segment containing that row. The
frozen-row band is stored first with `scroll_band_start` marking the split. A
full-grid candidate is compared with the committed tree to select:

- `Skip` for equal pixels;
- `Rows` for safe full-width changed bands;
- `Full` when border bleed, range shape, stale truth, or span cost makes a
  partial repaint unsafe.

Row-shift blits rotate only the scroll-row band; column shifts and damage
strips mark truth stale. Retained fingerprint truth is explicit: a commit
either installs a complete derived tree or marks the prior tree stale; it
never implies that a partial observation is exact.

### Blit safety

`RendererCore::prepare_blit` preflights the 1–2 candidate-derived address
strips before the single `Painter::blit`:

- a compatible cache shift stages the revealed strip;
- a cold cache or incompatible layout falls back to a full-grid replacement
  (traced as `BlitFallback`);
- any `BridgeFailed` recycles all prepared work and holds the whole attempt.

Only after every strip is clean does execution apply the single merged shift,
run the shared grid shell, and splice the strips into the shifted buffers.
Moving pixels before preflight would strand stale shifted pixels on a failed
attempt.

## Layers, painters, and surfaces

`Orchestrator<S>` owns `LayerBase<S, GridRenderer<S::P>>` and
`LayerBase<S, OverlayRenderer<S::P>>`. `LayerBase` is gate-free. `Surface`
owns a painter and provides `resize` and `present`; all drawing goes through
the unsealed `Painter` trait, with scroll copying isolated in `BlitPainter`.

Browser surfaces have different presentation semantics:

- grid paints into an opaque detached back canvas, reads blit source pixels
  from the visible front canvas, and copies back → front in `present`;
- overlay draws directly into a transparent/desynchronized canvas, so its
  `present` is a no-op.

Replay must present each replayed grid frame before a later recorded blit,
because the later blit reads the visible front canvas.

The shipped painters are `CanvasPainter`, `SvgPainter`, `PdfPainter`,
`RecorderPainter`, and the `RecordingPainter<P>` decorator. SVG measures and
draws embedded Inter; PDF measures and draws base-14 Helvetica; Recorder uses
the core approximation. `CanvasPainter` owns DPR, sticky setter caches,
palette interning, and a bounded `(font_css, text)` measurement memo.

## Overlay contract

Persistent model pixels belong on the grid. Transient interaction pixels
belong on the overlay. `Decorations` owns built-in and consumer layer order.
Committed selection and active-cell state is refreshed once in
`finish_attempt`, using the same captured sheet/view/selection visibility as
the frame being completed.

Content work may also require overlay paint because `ActiveCellRepaint`
contains model-derived pixels. The planner owns this CONTENT → OVERLAY rule;
regime helpers must not re-derive it independently.

## Query pipeline

The public query methods delegate through the facade to
`Orchestrator::last_frame`:

- `hit_test` classifies corner, headers, cells, autofill, and formula-ref
  handles;
- `cell_rect` and `range_rect` resolve visible geometry;
- `resize_handle_at` snaps to row/column trailing edges;
- `autofill_handle` derives the handle from committed selection state;
- `scroll_pane_rect`, `legal_scroll_origin`, and `scroll_to_show` expose
  renderer-owned viewport math for navigation consumers.

Before the first committed paint these return their absent variants. A held
attempt does not advance `last_frame`, so queries continue to match the
visible committed pixels.

## Adapter and format boundaries

`CanvasModel: CellContentQuery` is read-only. `IronCalcModel<'a>` exists
because orphan rules prevent implementing the core trait directly for
IronCalc's `UserModel`. The JS-backed adapter batches styles, values, and cell
types across the wasm boundary and preserves `Fetched::BridgeFailed` on its
degrade path; conditional-format decorations currently use the trait's
per-cell default loop.

`Surface` implementations for SVG and PDF are throwaway Fresh renderers, so
their `BlitPainter` implementations are no-ops and live viewport reuse must
never reach them.

The recorder's `.icr` schema is version 5. Each attempt stores a
recorder-owned `FrameTrace` projection: selected versus effective strategy,
work bits, one grid verdict (`GridVerdict`), whole-frame outcome, blit
fallback, attempt/commit identities, and fetch attribution. Non-idle zero-op
holds remain in the timeline.

## Source map

Paths are relative to `iron-canvas/crates/`.

| Source | Read for |
| --- | --- |
| `iron-canvas-core/src/frame_plan.rs` | `FrameInputs`, capture failures, `FrameDelta`, and `RebuildReason`. |
| `iron-canvas-core/src/pending_work.rs` | queued work algebra, row-span normalization, and `WorkFlags`. |
| `iron-canvas-core/src/orchestrator.rs` | `FramePlan`, five-strategy planner, attempt dispatch, `PaintOutcome`, `finish_attempt`, retry, queries, and trace publication. |
| `iron-canvas-core/src/chrome/` | committed geometry, stable classification, Fresh/SlotsReuse construction, reversible blit candidates, `GridLayout` piecewise address layout, and slot allocation recycling. |
| `iron-canvas-core/src/geometry/slot.rs` | axis-generic slot filling and pixel/id queries. |
| `iron-canvas-core/src/renderer/prepared.rs` | `FetchedCells`, `PreparedGrid`, `GridCacheAction`/`GridCacheCommit`, and the prepare/execute boundary. |
| `iron-canvas-core/src/renderer/cache/` | per-call scratch, `GridCache` segment buffers, fingerprint truth, and layout-transition classification. |
| `iron-canvas-core/src/renderer/cell/` | five-pass painting, fingerprint construction, repaint planning, borders, text, and conditional formatting. |
| `iron-canvas-core/src/renderer/mod.rs` | shared grid shell, strategy execution, trace collection, and layer-facing wrappers. |
| `iron-canvas-core/src/layer/mod.rs` | `Surface`, `LayerBase`, background/clear policy, and layer presentation boundary. |
| `iron-canvas-core/src/decoration/` | built-in overlay layers and z/hit-test order. |
| `iron-canvas-core/src/model_adapter.rs` | `CanvasModel`, `CellContentQuery`, bulk methods, and `Fetched<T>` semantics. |
| `iron-canvas-canvas2d/src/` | Canvas2D painter, paired runtime, measurement cache, theme bridge, and front/back canvas presentation. |
| `iron-canvas-web/src/orchestrator.rs` | wasm facade, DPR, scheduler result, recording bracket, and export entry points. |
| `iron-canvas-web/src/wasm/mod.rs` | JS model bridge, batch capability probes, failures, and theme caching. |
| `iron-canvas-web/src/playback.rs` | timed playback and per-frame grid presentation. |
| `iron-canvas-recorder/src/recording.rs` | `.icr` v5 attempt schema and trace projection. |
| `iron-canvas-export/src/` | SVG/PDF painters, metrics, document writers, and one-shot surfaces. |
| `iron-canvas-ironcalc/src/lib.rs` | IronCalc adapter and style/decoration conversion. |

## Verification

Use the narrowest gate that proves the changed boundary:

```bash
(cd iron-canvas && cargo test -p iron-canvas-core --locked)
(cd iron-canvas && cargo test --workspace --locked)
(cd iron-canvas && cargo check --target wasm32-unknown-unknown --locked)
(cd iron-canvas && \
  cargo test --target wasm32-unknown-unknown \
    -p iron-canvas-web -p iron-canvas-datagrid-web --locked)
```

The wasm test command requires the lockfile-matched
`wasm-bindgen-test-runner` and ChromeDriver configuration used by
`.github/workflows/test.yml`. Browser retained-pixel tests compare raw
Canvas2D `ImageData` byte-for-byte with forced-Fresh output. Those tests prove
only their named held Fresh/Viewport recovery, row/border repaint,
stable-geometry edit, and post-blit cases; new retained-pixel behavior needs
its own forced-Fresh comparison.

For documentation maintenance, run `scripts/arch-staleness.sh`, then also
inspect `git status`, unstaged/staged diffs, untracked files, and dirty
submodules: the scanner only compares the recorded anchor with `HEAD`.
