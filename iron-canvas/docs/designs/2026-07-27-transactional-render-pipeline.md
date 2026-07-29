# Transactional Render Pipeline — Architecture Revision

**Date:** 2026-07-27
**Status:** Draft — source audit complete; no production changes implemented
**Reviewed source:** `canvas-api-cleanup` at `e79416d`

**Update (2026-07-29):** Stage 0-1 landed, see
`docs/superpowers/plans/2026-07-28-transactional-stage-0-1.md`.

## Summary

The five paint regimes are not the main problem. They are useful names for
different costs. The problem is that one render attempt is represented by
several overlapping decisions and has no single commit boundary.

Today, dirty intent is split between two `PaintGate`s, `GridSignals`,
`pending_content`, `pending_damage`, live geometry comparisons, and the
previous selection snapshot. A chosen regime then becomes a `FramePath`,
`BlitOutcome`, `FrameKindTag`, `stale_panes`, pane-cache invalidation, and one
of three renderer entry points. Failure is observed inside the renderer, while
presentation, `last_frame`, dirty cleanup, retry, and diagnostics remain owned
by callers that cannot see the failure.

The revision should keep the current paint strategies but put them inside one
transaction:

```text
PendingWork -> FramePlan -> prepare -> execute -> commit or hold
```

Only a successful commit may present surfaces, advance `last_frame`, consume
pending work, or stamp the effective trace. A held attempt keeps the previous
painted frame and retains enough work to retry.

## Scope

This design covers:

- dirty and damage accumulation;
- frame-delta classification;
- paint-strategy planning;
- bridge-failure preparation and retry;
- grid/overlay presentation;
- committed-frame/query coherence;
- effective frame tracing;
- Canvas2D resize and playback presentation.

It deliberately does not redesign:

- pane fingerprints or their hash domain;
- the five-pass cell painter;
- row-damage safety rules;
- pane geometry and slot walks;
- the `Painter` abstraction;
- the read-only `CanvasModel` boundary.

Further fingerprint optimization should wait until this lifecycle is explicit.

## Source audit

### Blocker 1 — a held viewport attempt is still committed

The inner blit preflight is atomic, but the outer frame lifecycle is not.

`RendererCore::prefetch_blit_strips` returns `false` before shifting pixels and
records `HeldOnBridgeFailure`:

- `crates/iron-canvas-core/src/renderer/cell/mod.rs:466-537`

`LayerBase::paint_grid_blit` converts that result into an early `return`, but
its return type is `()`:

- `crates/iron-canvas-core/src/layer/mod.rs:182-202`

The viewport arm cannot see whether painting happened. It still:

1. calls `grid.present()`;
2. refreshes and paints the overlay against the candidate frame;
3. presents the overlay;
4. stores the candidate as `last_frame`.

Evidence:

- `crates/iron-canvas-core/src/orchestrator.rs:816-842`

`paint_if_dirty` then clears `pending_content` and `pending_damage`
unconditionally:

- `crates/iron-canvas-core/src/orchestrator.rs:669-716`

For Canvas2D, grid `present()` copies the unchanged back buffer to the visible
front canvas:

- `crates/iron-canvas-canvas2d/src/web_surface.rs:93-100`

The resulting state can be:

```text
grid pixels     previous frame
overlay pixels  candidate frame
last_frame      candidate frame
hit testing     candidate frame
pending work    cleared
```

That contradicts the current `ARCHITECTURE.md` claim that `last_frame` is
always the snapshot the last paint emitted.

The existing blit failure test proves only inner renderer atomicity. It does
not exercise orchestrator presentation, overlay painting, `last_frame`, or
retry:

- `crates/iron-canvas-core/tests/scroll_blit.rs:612-692`

### Blocker 2 — resize mutates backing stores without invalidating

`Orchestrator::resize` updates size, resizes both surfaces, and updates painter
DPR state, but raises no dirty work:

- `crates/iron-canvas-core/src/orchestrator.rs:319-325`

Canvas2D resize can reallocate and therefore clear both the visible and
detached backing stores:

- `crates/iron-canvas-canvas2d/src/web_surface.rs:76-90`

If no separate signal was raised, `paint_if_dirty` drains nothing and returns.
The RustyCalc worksheet compensates with a separate `request_repaint()` call:

- `src/components/workbook/worksheet/mod.rs:65-101`

The standalone `DataGridCanvas::resize` does not:

- `crates/iron-canvas-datagrid-web/src/lib.rs:119-125`

Resize, including a DPR-only change, must be an atomic geometry invalidation.
Callers should not need to remember a second method.

### Blocker 3 — playback writes the back canvas but never presents it

The grid `CanvasPainter` targets the detached back canvas:

- `crates/iron-canvas-canvas2d/src/web_surface.rs:31-42`

Playback replays recorded operations directly through the painters:

- `crates/iron-canvas-web/src/playback.rs:103-134`
- `crates/iron-canvas-web/src/orchestrator.rs:1000-1014`

It never calls the surfaces' `present()`. Normal `paintIfDirty` also
short-circuits during playback:

- `crates/iron-canvas-web/src/orchestrator.rs:221-245`

The overlay may visibly update because it draws directly, while the grid
remains the previous live front-canvas image. Playback needs a surface-level
replay/present boundary and a browser pixel test.

### High — failed content work is consumed instead of retried

Normal pane and damage-strip paths correctly preserve prior pixels and caches
when a bulk fetch contains `BridgeFailed`:

- full pane: `crates/iron-canvas-core/src/renderer/cell/mod.rs:147-166`;
- damage strip: `crates/iron-canvas-core/src/renderer/cell/mod.rs:790-837`.

However, the renderer reports only a trace verdict. The orchestrator still
presents, commits, and clears the only content notification. Under the
demand-driven rAF loop, no automatic next tick exists.

The damage recovery test explicitly raises a second damage notification after
the bridge recovers:

- `crates/iron-canvas-core/tests/orchestrator_regimes.rs:703-744`

This proves cache survival, not an engine retry contract.

### High — dirty intent has no single representation

The same content work is spread across:

- two layer-owned `PaintGate`s;
- `GridSignals::CONTENT`;
- `pending_content: PaneRegionMask`;
- `pending_damage: CellDamage`;
- `PaintRegime::{Damage, SlotsReuse}`;
- `FramePath::SlotsReuse { stale_panes }`;
- `Chrome.stale_panes`;
- explicit `PaneCache::invalidate(mask)`;
- renderer pane iteration.

For the slots-reuse path, one pane mask travels through most of those forms:

- accumulation and planning:
  `crates/iron-canvas-core/src/orchestrator.rs:255-263, 723-784`;
- frame construction:
  `crates/iron-canvas-core/src/chrome/mod.rs:198-220`;
- cache invalidation and paint:
  `crates/iron-canvas-core/src/orchestrator.rs:894-915`;
- renderer iteration:
  `crates/iron-canvas-core/src/renderer/mod.rs:254-267`.

Damage exposes the mismatch: the orchestrator builds a slots-reuse frame with
`stale_panes: EMPTY`, then the damage renderer ignores that field and visits
all panes:

- `crates/iron-canvas-core/src/orchestrator.rs:852-870`;
- `crates/iron-canvas-core/src/renderer/mod.rs:373-385`.

Paint work does not belong on `Chrome`; `Chrome` should remain painted
geometry.

### High — frame validity and blit screening re-read the same state

`Chrome::is_still_valid` and `Chrome::screen_for_blit` independently read and
compare size, theme, selected view, sheet, and frozen counts:

- `crates/iron-canvas-core/src/chrome/mod.rs:376-465`

If rebuilding follows, `Chrome::build` reads much of it again:

- `crates/iron-canvas-core/src/chrome/mod.rs:263-373`

Overlay refresh then reads the selected view and active-cell value again:

- `crates/iron-canvas-core/src/decoration/selection.rs:30-50`

The duplication costs bridge calls and makes one paint attempt depend on
several independently sampled views of model state. Capture the frame-level
inputs once, then classify and build from that snapshot.

### High — selected regime and executed path can disagree

`PaintRegime::Viewport` is stamped before execution:

- `crates/iron-canvas-core/src/orchestrator.rs:689-713`

`Chrome::next_blit` can then return `BlitOutcome::FreshFallback`, causing a
full fresh paint:

- `crates/iron-canvas-core/src/orchestrator.rs:816-830`

The trace still says `Viewport`. Separately, `FrameKindTag` controls background
clearing, cache reuse, and staged-fetch adoption downstream:

- `crates/iron-canvas-core/src/layer/mod.rs:169-180`;
- `crates/iron-canvas-core/src/renderer/cell/mod.rs:85-103, 558-570`.

Trace should distinguish:

- selected strategy;
- effective execution path;
- commit outcome.

### High — overlay completion policy is duplicated and has drifted

Every regime arm separately refreshes, decides, paints, and presents overlay
state:

- Overlay: `crates/iron-canvas-core/src/orchestrator.rs:787-804`;
- Viewport: `crates/iron-canvas-core/src/orchestrator.rs:806-842`;
- Damage: `crates/iron-canvas-core/src/orchestrator.rs:845-885`;
- SlotsReuse: `crates/iron-canvas-core/src/orchestrator.rs:888-938`;
- Fresh: `crates/iron-canvas-core/src/orchestrator.rs:941-968`.

Damage and SlotsReuse implement the `CONTENT -> active-cell overlay` rule.
Fresh only checks `signals.overlay_dirty()`. A content change combined with
geometry divergence can therefore fresh-paint the grid while preserving stale
model-derived active-cell overlay pixels.

The previous active-cell snapshot used for blit safety also lives inside the
selection decoration:

- `crates/iron-canvas-core/src/orchestrator.rs:740-755`;
- `crates/iron-canvas-core/src/decoration/selection.rs:13-20`.

That snapshot is committed-frame safety state, not decoration policy.

### Medium — the renderer repeats its load-bearing shell

Normal, blit, and damage rendering each repeat:

```text
Grid -> Cells -> FrozenSep -> Headers -> Corner
```

Evidence:

- normal: `crates/iron-canvas-core/src/renderer/mod.rs:254-288`;
- blit: `crates/iron-canvas-core/src/renderer/mod.rs:290-371`;
- damage: `crates/iron-canvas-core/src/renderer/mod.rs:373-404`.

Frozen-separator ordering and header visibility are correctness rules. They
should live in one scaffold parameterized by cell work and header scope.

### Medium — the public wake-up vocabulary does not match intent

`GridSignals::VIEWPORT` is reserved and never raised:

- `crates/iron-canvas-core/src/signal.rs:1-19`

Navigation calls `request_overlay_repaint`, which wakes the dispatcher so it
can rediscover a possible viewport shift geometrically:

- `src/components/workbook/worksheet/subscribe.rs:115-120`

The method name describes a target layer while its real contract is “view or
selection may have changed.” The engine needs a typed `view_changed`
notification. Geometry comparison should remain as a correctness check, not as
the only representation of intent.

### Medium — diagnostics cannot say whether this call painted

Core and facade `paint_if_dirty` both return `()`:

- `crates/iron-canvas-core/src/orchestrator.rs:665-717`;
- `crates/iron-canvas-web/src/orchestrator.rs:221-245`.

`frameTrace()` always returns the previous trace. RustyCalc samples it after
every poked callback and increments a painted-frame counter even if the call
was idle or playback short-circuited:

- `src/components/workbook/worksheet/raf_loop.rs:182-205, 242-255`.

A typed paint result should drive retries, diagnostics, and scheduler
continuation.

## Architecture-document drift

`scripts/arch-staleness.sh` reports both architecture documents stale. The
canvas document is marked verified at `c16e104`; live HEAD is `e79416d`.

Confirmed contradictions include:

- `ARCHITECTURE.md` says overlay layers read live state with no refresh step,
  but `Decorations::refresh_overlay_state` is called by the orchestrator;
- it says `set_theme` drops `last_frame` and validity does not check theme,
  while live `set_theme` preserves the frame and
  `Chrome::is_still_valid` compares themes;
- its four-input validity list omits theme;
- its paint/query coherence claim does not account for held attempts;
- `layer::Surface` documentation calls Canvas2D presentation a no-op even
  though grid `present()` is the load-bearing back-to-front copy.

`docs/rendering-and-damage.md` is also stale: it says RustyCalc does not call
`markRowsDamaged`, while the current subscription routes
`CellChanged`/`RangeChanged` through it.

Do not patch the canonical documents to describe the proposed architecture
before implementation. Update them stage by stage from landed code.

## Proposed ownership

### 1. `Orchestrator` owns one `PendingWork`

Replace layer-owned dirty gates plus side-band content state with one value:

```rust
#[derive(Default)]
struct PendingWork {
    geometry: GeometryWork,
    view: bool,
    content: ContentWork,
    overlay: bool,
}

#[derive(Default)]
enum ContentWork {
    #[default]
    Clean,
    Rows {
        sheet: u32,
        spans: Vec<RowSpan>,
    },
    Panes(PaneRegionMask),
}
```

Merge rules live on `ContentWork`:

- compatible row damage merges spans;
- cross-sheet, unscoped, or over-cap rows degrade to `Panes(ALL)`;
- `Clean` is the absence of content work, so no separate `CONTENT` bit can
  disagree with it.

Setters update `PendingWork` atomically:

- `resize(size, dpr)` -> geometry;
- `set_model` -> geometry + content all + overlay;
- canvas palette change -> geometry + overlay;
- model theme change/fonts change -> content all;
- `mark_rows_damaged` -> rows;
- `mark_content_dirty` -> panes;
- `view_changed` -> view + overlay;
- overlay setters -> overlay.

`request_repaint` remains a conservative recovery escape hatch, not routine
host glue.

### 2. capture frame-level model inputs once

Introduce a small immutable input snapshot for scalar frame state:

```rust
struct FrameInputs {
    size: CanvasSize,
    dpr: f64,
    theme: Rc<CanvasTheme>,
    view: CanvasView,
    sheet: u32,
    frozen_rows: i32,
    frozen_cols: i32,
    show_row_headers: bool,
    show_col_headers: bool,
}
```

The snapshot does not copy row/column extents or cell data. Slot walks and pane
fetches remain lazy. It ensures frame classification, build, and overlay
snapshot use one selected view/sheet/freeze sample.

If scalar inputs cannot be captured because the bridge fails, the attempt is
held. Do not build and commit the synthetic default A1 frame currently used by
`Chrome::build`.

### 3. one classifier produces a `FrameDelta`

Replace the overlapping `is_still_valid` and `screen_for_blit` calls with:

```rust
enum FrameDelta {
    Stable,
    Scroll(BlitPlan),
    Rebuild(RebuildReason),
}
```

The classifier compares the committed frame to `FrameInputs` once. Blit
qualification remains conservative and may return `Rebuild`.

`RebuildReason` is diagnostic. It should name size/DPR, theme, model, sheet,
freeze, header, two-axis scroll, and incompatible overlap without changing
correctness behavior.

### 4. planning produces one `FramePlan`

Keep the five useful strategies, but carry all execution work in the plan:

```rust
enum GridWork {
    None,
    Fresh,
    Panes(PaneRegionMask),
    Rows(Vec<RowSpan>),
    Blit(PreparedBlitPlan),
}

enum OverlayWork {
    Preserve,
    Paint,
}

struct FramePlan {
    selected_strategy: PaintStrategy,
    candidate: CandidateFrame,
    grid: GridWork,
    overlay: OverlayWork,
    consumes: PendingWork,
}
```

Rules:

- `Chrome` contains geometry only; remove `stale_panes`;
- `FrameKindTag` becomes diagnostic or is replaced by the effective strategy;
- pane masks, row spans, background policy, and header scope belong to
  `FramePlan`/`GridWork`;
- overlay policy is calculated once from the candidate geometry, content
  coupling, and overlay changes.

### 5. preparation is side-effect free

Any bridge-dependent fetch required to prove that a strategy can execute is
prepared before pixel or cache mutation.

For blit, do not classify pane work twice. The current preflight and execution
both derive shift work. Store the typed result:

```rust
struct PreparedBlitPane {
    work: BlitPaneWork,
    fetched: FetchedCells,
    cache_action: PaneCacheAction,
}
```

Use one named `FetchedCells` bundle for the four channels instead of repeating
the same tuple across pane buffers, blit staging, and damage scratch.

The 2026-06-14 cache/blit design already specified the missing lifecycle:
prepare, paint, commit on success; keep the previous frame, dirty intent, and
retry on failure. Current code implemented much of its typed pane work and
preflight, but not the outer `PaintStatus`/commit/retry boundary.

### 6. execution returns an outcome

```rust
enum PaintOutcome {
    Committed {
        painted_layers: PaintedLayers,
        effective_strategy: PaintStrategy,
        frame: Chrome,
        trace: FrameTrace,
    },
    Partial {
        painted_layers: PaintedLayers,
        frame: Chrome,
        retry: PendingWork,
        trace: FrameTrace,
    },
    Held {
        retry: PendingWork,
        trace: FrameTrace,
    },
}
```

Viewport blit is whole-frame atomic: any failed prepared pane returns `Held`.

Normal pane and row damage may use `Partial` if successful panes can be
presented safely while failed panes retain old pixels. The failed pane
mask/spans must be retained for retry. If partial semantics remain difficult
to prove, hold the whole grid attempt first; correctness before granularity.

### 7. one commit function owns completion

Only the commit step may:

- present the grid or overlay;
- update pane ranges and fingerprint state representing painted pixels;
- advance `last_frame`;
- consume or merge pending work;
- stamp effective trace state;
- tell the scheduler whether a retry is required.

```text
Committed -> present painted layers
          -> store committed frame
          -> consume planned work
          -> stamp effective trace

Partial   -> present safe layers
          -> store coherent frame
          -> requeue failed scope
          -> stamp partial trace

Held      -> present nothing
          -> keep previous frame
          -> requeue work
          -> stamp held attempt
```

Overlay refresh and paint should be one common post-grid step, driven by the
actual outcome. A held viewport must not paint the overlay against its
candidate frame.

### 8. return a scheduler-facing result

```rust
enum PaintResult {
    Idle,
    Painted(FrameTrace),
    Retry(FrameTrace),
}
```

Core and wasm facades should return this result or a small wire equivalent.
RustyCalc's one-shot rAF closure remains active when playback is running or
the engine returns `Retry`.

Diagnostics update only for `Painted`/`Retry`, never from stale trace state.
Trace fields should include:

- monotonically increasing attempt/commit sequence;
- selected strategy;
- effective strategy;
- commit outcome;
- failed pane scope;
- fetched cell-slot count.

### 9. Canvas2D owns paired surface lifecycle

A Canvas2D-specific pair/runtime should own:

- grid and overlay surfaces;
- grid and overlay painter handles;
- viewport metrics (`CanvasSize` + DPR);
- resize;
- font measurement-cache invalidation;
- presentation;
- playback replay/present.

This removes repeated two-painter/font/resize wiring from `IronCanvas`,
`DataGridCanvas`, and `CameraCanvas` while keeping the generic core free of
browser APIs.

## Target flow

```text
host mutation
    |
    +--> typed invalidation + scheduler poke
              |
              v
        PendingWork::merge
              |
              v
        capture FrameInputs
              |
              v
        classify FrameDelta
              |
              v
        build FramePlan
              |
              v
        prepare fetch/cache work
              |
        +-----+------+
        |            |
      clean        failed
        |            |
        v            v
      execute       Held
        |
        v
      commit
        |
        +--> present
        +--> last_frame
        +--> cache/fingerprint commit
        +--> pending-work consume/requeue
        +--> effective trace
        +--> Painted / Retry
```

Queries read only the last committed `Chrome`.

## File impact

| File | Action | Responsibility change |
| --- | --- | --- |
| `crates/iron-canvas-core/src/orchestrator.rs` | Modify/split | Own `PendingWork`, planning, outcome, and commit; remove five duplicated completion tails. |
| `crates/iron-canvas-core/src/signal.rs` | Replace/refocus | Replace bits + side-band invariants with typed pending work and merge rules. |
| `crates/iron-canvas-core/src/chrome/mod.rs` | Modify | Accept captured frame inputs; expose one delta classifier; keep `Chrome` geometry-only. |
| `crates/iron-canvas-core/src/chrome/blit.rs` | Modify | Keep pixel geometry planning; return prepared data to the frame planner. |
| `crates/iron-canvas-core/src/layer/mod.rs` | Modify | Return typed execution outcomes; never hide a held paint behind `()`. |
| `crates/iron-canvas-core/src/renderer/mod.rs` | Modify | Execute one grid scaffold from `GridWork`; consume prepared blit work once. |
| `crates/iron-canvas-core/src/renderer/cell/mod.rs` | Modify | Return failed scope; use a named fetched-data bundle. Preserve five-pass paint order. |
| `crates/iron-canvas-core/src/renderer/cache/pane_cache.rs` | Modify | Separate prepare from commit; cache metadata means committed pixels. |
| `crates/iron-canvas-canvas2d/src/` | Add/modify | Introduce paired surface lifecycle for resize, fonts, present, and playback. |
| `crates/iron-canvas-web/src/orchestrator.rs` | Modify | Return paint status; route playback through surface presentation. |
| `crates/iron-canvas-datagrid-web/src/lib.rs` | Modify | Remove caller-side resize assumptions; use typed view/geometry notifications. |
| `src/components/workbook/worksheet/` | Modify | Consume `PaintResult`; remove redundant resize repaint and overlay-as-viewport wake-up. |
| `iron-canvas/ARCHITECTURE.md` | Update per stage | Rewrite around ownership and transaction after the code lands. |
| `iron-canvas/docs/rendering-and-damage.md` | Update per stage | Correct current routing and retry semantics. |
| `iron-canvas/AGENTS.md` | Update | Remove stale pass-count, schema, DPR, theme, and presentation claims. |

## Migration order

### Stage 0 — pin the broken boundaries

Add failing tests before refactoring:

1. held viewport keeps the previous query geometry and presents neither layer;
2. held viewport retains work and retries after bridge recovery;
3. held Damage/SlotsReuse retains failed content scope;
4. resize alone causes a Fresh repaint, including DPR-only resize;
5. playback seek updates visible front-canvas pixels;
6. CONTENT plus Fresh repaints the active-cell overlay;
7. trace distinguishes selected Viewport from effective Fresh fallback.

Do not start fingerprint rotation in this stage.

### Stage 1 — make outcomes visible

- return a typed result from blit/full/damage layer paint methods;
- propagate failed pane scope;
- make `paint_if_dirty` return `PaintResult`;
- stop committing/presenting held viewport attempts;
- retain/requeue content work on failure;
- fix playback presentation.

This is the smallest correctness repair. It may temporarily retain the current
five regime arms and dirty representation.

### Stage 2 — make pending work one value

- introduce `PendingWork` and `ContentWork`;
- migrate setters one by one;
- make resize atomically enqueue geometry work;
- add `view_changed`;
- remove the two layer `PaintGate`s, `GridSignals::VIEWPORT`, and the
  `pending_content <=> CONTENT` side invariant.

Run regime tests after each setter group.

### Stage 3 — capture inputs and produce one plan

- add `FrameInputs`;
- replace separate validity/blit screens with `FrameDelta`;
- add `FramePlan`/`GridWork`/`OverlayWork`;
- move `stale_panes` out of `Chrome`;
- stamp selected and effective strategy separately.

### Stage 4 — unify prepare/execute/commit

- make bridge fetch preparation side-effect free;
- consume prepared blit classification once;
- centralize overlay completion;
- centralize presentation, frame commit, pending cleanup/requeue, and tracing;
- decide whether non-blit failures are whole-frame Held or safe Partial.

### Stage 5 — unify the renderer shell

- parameterize the common Grid/Cells/FrozenSep/Headers/Corner sequence;
- keep pane strategies and five-pass cell paint as focused inner operations;
- introduce named fetched-data bundles where they remove tuple plumbing.

### Stage 6 — simplify only with measurements

After correctness and trace tests are green:

- measure duplicate cache invalidation;
- measure production value of cell fingerprint leaves;
- evaluate row-axis fingerprint rotation;
- evaluate fetch traffic separately from painter traffic.

Do not delete cache or fingerprint state based only on code reading.

### Stage 7 — rewrite architecture documentation

Make `ARCHITECTURE.md` reader-first:

1. ownership table;
2. one render-attempt lifecycle;
3. typed pending work;
4. five strategy summaries;
5. commit/retry invariant;
6. query coherence;
7. crate boundaries.

Move the exhaustive file inventory to a separate reference or generated
appendix. The canonical document should explain where decisions are owned,
not repeat every field and historical implementation detail.

## Tests

### Native core

- planner table tests for every `PendingWork x FrameDelta` combination;
- merge/property tests for `ContentWork`;
- exhaustive selected/effective strategy tests;
- held/partial/committed cache and frame-state tests;
- common renderer-shell ordering tests with `RecorderPainter`;
- active-cell overlay coupling across every grid strategy.

### Canvas2D browser

- held blit preserves front pixels and query geometry;
- successful retry matches forced-Fresh `ImageData`;
- resize/DPR repaint matches forced Fresh;
- playback seek updates front `ImageData`;
- retained-pixel changes continue to use scenario-specific forced-Fresh
  comparisons.

### Scheduler

- `Idle` pauses one-shot rAF;
- `Painted` pauses unless playback remains active;
- `Retry` keeps or re-arms the loop;
- redundant host events do not publish stale frame traces.

## Trade-offs

### Benefits

- correctness is enforced at a commit boundary instead of by comments;
- one value explains pending work;
- queries cannot advance past pixels;
- transient bridge failures have a retry contract;
- resize and playback own their presentation effects;
- traces describe what executed, not only what was selected;
- five strategies remain available without five copies of completion logic;
- later performance work gets a stable measurement boundary.

### Costs

- preparation may retain staged fetch buffers until commit;
- typed plans/outcomes add several small enums;
- migrating dirty setters touches core, facades, and hosts;
- safe partial commit needs careful proof; whole-frame hold may be simpler
  initially;
- changing wasm paint return values requires wire/API coordination.

### Rejected alternatives

- **Collapse everything to Fresh.** Simple but discards the proven value of
  overlay, blit, damage, and fingerprint strategies.
- **Add more signal bits.** Does not solve split side-band state or commit
  ownership.
- **Fix only the blit early return.** Leaves non-blit retry loss, resize,
  playback, overlay drift, and misleading traces.
- **Move all policy into the renderer.** Violates the useful boundary:
  orchestrator plans; renderer executes paint work.
- **Update `ARCHITECTURE.md` first.** Would describe an architecture the code
  does not yet enforce.

## Review checklist

- [ ] Canvas remains read-only against `CanvasModel`.
- [ ] `Painter` remains the only drawing boundary.
- [ ] `Chrome` represents committed geometry, not pending paint work.
- [ ] No failed attempt advances queries or presents a candidate frame.
- [ ] Pending content scope cannot disagree with a separate dirty bit.
- [ ] Resize owns geometry invalidation, including DPR-only changes.
- [ ] Playback reaches surface presentation.
- [ ] Overlay completion is decided once for every effective grid path.
- [ ] Closed strategy/outcome enums are matched exhaustively.
- [ ] `BridgeFailed` remains distinct from an absent cell.
- [ ] Blit fetches complete before any pixel/cache mutation.
- [ ] Five-pass cell order remains background -> CF -> grid border ->
      explicit border -> text.
- [ ] Performance claims are measured and retained-pixel claims have
      browser `ImageData` comparisons against forced Fresh.

## Recommendation

Approve Stages 0-1 as the immediate repair. Review the `PendingWork` and
`FramePlan` type shapes before starting Stage 2. Defer the current
fingerprint-tree alignment/rotation work until the transaction and trace
boundaries can prove whether the remaining cost is fetch, planning, or paint.
