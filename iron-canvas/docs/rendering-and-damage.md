# Rendering, caches, and damage detection — how a repaint is decided

This document explains the runtime behaviour of the iron-canvas paint
pipeline: what is cached, how the engine decides *what* to repaint, what
actually rerenders when you edit a cell, and the modeling rules that keep
these optimizations maintainable. Frame construction, slot vecs, and the
query API are covered in `ARCHITECTURE.md`; this doc is about the
*decision machinery* on top of them.

Sources of truth: `frame_plan.rs` (`FrameInputs` capture, `FrameDelta`),
`chrome/mod.rs` (`Chrome::classify`), `orchestrator.rs` (`plan_frame` +
dispatch + `finish_attempt`), `renderer/prepared.rs` (`FetchedCells`,
`Prepared*` types — the prepare/execute transaction boundary),
`pending_work.rs` (queued work), `renderer/cell/mod.rs` +
`renderer/cell/fingerprint.rs` (data compare), `renderer/cache/pane_cache.rs`
(cross-frame buffers), `iron-canvas-canvas2d/src/web_surface.rs` (double
buffering).

---

## 1. The cost ladder — five regimes

Every rAF tick calls `Orchestrator::paint_if_dirty`. It takes the single
queued `PendingWork` value (`self.pending`, via one `mem::take` — no
layer holds dirty state of its own), then runs it through the full
pipeline: `PendingWork -> FrameInputs -> FrameDelta -> FramePlan -> prepare
-> execute -> finish`. The first three stages are pure planning (points
1-3 below); `prepare`, `execute`, and the single `finish` completion
boundary (point 4) are the transactional guarantee this document exists
to explain.

1. **`FrameInputs::capture`** reads every scalar the frame needs —
   selected sheet, selected view, frozen row/column counts, header
   visibility, selection visibility — exactly once, before any geometry
   walk or paint runs. A failure here holds the *entire* attempt (see
   "Capture-failure hold" below) instead of falling back to a synthetic
   default.
2. **`Chrome::classify(prev, model, inputs, active_cell)`** compares the
   captured inputs against the previously committed `Chrome` — itself
   nothing more than committed geometry plus the classifier-relevant
   metadata (canvas size, DPR, theme, model generation, sheet, frozen
   counts, header visibility, effective scroll origin); it carries no
   pending paint scope of its own. `classify` returns one `FrameDelta`:
   `Stable` (nothing moved), `Scroll(BlitPlan)` (a geometrically safe
   single-axis shift), or `Rebuild(RebuildReason)` (a named hard break).
3. **`plan_frame(work, delta, sheet, show_selection)`** is a pure
   function turning the taken `PendingWork` plus the `FrameDelta` into
   one closed `FramePlan { selected_strategy, grid, overlay, consumes,
   rebuild_reason }`. `grid: GridWork` — `None | Fresh | Panes(mask) |
   Rows{sheet,spans} | Blit(plan)` — carries the pane scope explicitly
   into the executor it dispatches to; `Chrome` itself is never
   consulted for which panes need repainting.
4. **Prepare, execute, finish.** `FramePlan.grid` dispatches to one of
   five `paint_*_regime` methods. Each one *prepares* its own scope —
   every bulk bridge read (`FetchedCells::fetch_into`, in
   `renderer/prepared.rs`) classified against the pane cache's
   *committed* `painted` fingerprint tree — without writing any
   committed cache/pixel/frame state, then *executes*: paints the
   prepared healthy scope into the backing target and returns its owned
   `PreparedCacheCommit`. Execution then reduces to one `PaintOutcome`
   (`Committed` / `Partial` / `Held`); Committed and Partial carry only
   healthy panes' cache entries, while Held carries none.
   `Orchestrator::finish_attempt` is the single completion boundary
   every outcome flows through — it installs that aggregate once, advances
   or preserves `last_frame`, presents whichever layers actually painted,
   merges retry work back into `self.pending`, and publishes the frame's
   trace.

`PendingWork` tracks four categories — geometry rebuild, view movement,
content damage (rows or panes), overlay repaint — and the diagnostic
`WorkFlags` bitflags project them as `VIEW | CONTENT | GEOMETRY |
OVERLAY` for tracing and recording. `plan_frame`'s table is ordered
cheapest-first, and `selected_strategy: PaintRegimeTag` — the same
five-variant tag recorded to `.icr` files — names the arm that wins:

| Regime       | Grid pixels touched                     | Model data fetched            | When |
|--------------|------------------------------------------|-------------------------------|------|
| `Overlay`    | none — grid canvas untouched             | none (overlay state only)     | selection move, autofill drag, marching ants |
| `Viewport`   | one revealed strip per shifted pane      | strip cells only              | scroll/arrow-key viewport shift, no content change |
| `Damage`     | named full-width row bands               | band cells only               | content change whose rows are all known (`ContentWork::Rows`) |
| `SlotsReuse` | changed row bands within changed panes (fingerprint tree + row-damage plan) | full bulk refetch, masked panes | content change, rows unknown; viewport otherwise reusable |
| `Fresh`      | everything                               | everything                    | first paint, resize, sheet/freeze change, theme swap, `request_repaint`, or content + view together |

`plan_frame` actually probes the geometric scroll delta *before* falling
back to `Overlay` — a view-only attempt that turns out to be a real
pixel shift lands `Viewport`; a view-only attempt that stays inside the
painted frame (ordinary arrow-key selection) falls through to `Overlay`
instead. `view_changed()` / `viewChanged()` marks *intent* only; whether
that intent becomes a blit, an overlay-only repaint, or a full rebuild is
`Chrome::classify`'s geometric verdict, never the caller's choice.

Four properties make the pipeline safe rather than optimistic:

- **Classification and planning are pure.** `Chrome::classify` only
  reads (`prev`, the model, captured inputs, the active-cell snapshot);
  `plan_frame` is a free function over owned values. Neither builds a
  `Chrome`, mutates cache state, invalidates a cache, or calls a
  painter. You can read the whole policy in two functions without
  chasing side effects.
- **Preparation is the only fallible step, and it never mutates
  committed state.** A `paint_*_regime` method's own bulk bridge reads
  run against the model and the pane cache's committed `painted`
  fingerprint tree, but may only write renderer-lifetime scratch
  (`PaneBuffers::prepare_scratch`, `FrameCache::strip_scratch`) — never
  `PaneBuffers`' content fields, `PaneBuffers::range`, or
  `PaneFingerprintState::painted`. A failed preparation is therefore
  always a safe no-op against everything a later frame's paint-skip or
  blit-shift decision reads.
- **The decision is a value, twice over.** `Chrome::classify` returns
  `FrameDelta` (`#[must_use]` by construction — every variant is
  matched exhaustively); `plan_frame` returns `FramePlan`, whose
  `grid: GridWork` field `paint_if_dirty` matches on to dispatch.
  Adding a `GridWork` variant breaks the build at the dispatch site by
  design.
- **Execution and completion are a value too.** Once preparation
  succeeds, execution is infallible: it paints the prepared scope and
  reduces to one `PaintOutcome`, never installing cache state,
  advancing `last_frame`, presenting a surface, or touching
  `self.pending` itself — `finish_attempt` is the one place that does.
  `paint_if_dirty` returns the resulting `PaintResult::{Idle, Painted,
  Retry}` (deliberately not `#[must_use]`: a permanent polling loop
  that ignores it still behaves correctly). `Retry` means an attempt
  was held back rather than committed, and how far back is resolved per
  regime, not left to the caller to infer:

  | Prepared work | On bridge failure | Retry scope |
  | --- | --- | --- |
  | `Overlay` (no bulk grid prepare) | not applicable | — |
  | `Viewport`, or selected-`Viewport`/effective-`FreshFallback` | whole-frame **Held** — `prepared.rollback()` moves `prev`'s untouched slot/header vectors back out of the candidate; no `Chrome::clone` | complete consumed work |
  | `Fresh` | whole-frame **Held** — `last_frame` is never taken (ordinary Fresh), or handed back exactly as `prepare_blit` returned it (`FreshFallback`) | complete consumed work |
  | `SlotsReuse` | **Partial** if any target pane painted (healthy panes present; failed panes keep prior pixels), else whole-frame **Held** | failed pane mask only (complete consumed work on a total hold) |
  | `Damage` | **Partial** if any intersected pane painted, else whole-frame **Held** | original sheet + row spans (complete consumed work on a total hold) |

  `Fresh` and `FreshFallback` are whole-frame atomic on bridge
  failure — never partial — because a Fresh candidate's geometry and
  full-canvas background may not agree with the committed frame, so
  preserving old pixels in just the failed panes would not be coherent
  with the new `Chrome`. `SlotsReuse` and `Damage` may commit partially
  because their geometry *is* the committed frame — planning already
  proved it stable — so a healthy pane's pixels occupy exactly the
  region a failed pane leaves untouched. Either way, the held or
  partial scope is merged back into `self.pending` before
  `finish_attempt` returns, so the caller can just call `paint_if_dirty`
  again next tick with no new external input needed.

This is the pipeline's actual, landed shape — not a plan. Consolidating
the five pane-quadrant renderer scaffolds (`Grid`/`Cells`/`FrozenSep`/
`Headers`/`Corner`) into one shared structure, and any fetch- or
measurement-driven performance work, are explicitly out of scope here:
future work, not described by this document.

### Capture-failure hold

`FrameInputs::capture` reads, in fixed order: selected sheet, selected
view (asserted to agree with the sheet), frozen row count, frozen
column count, row-header visibility, column-header visibility. Each is
an `Option`-returning `CanvasModel` accessor — including
`get_selected_sheet() -> Option<u32>`, changed from a bare `u32` so a
JS-bridge throw is observable as `None` instead of silently defaulting
to sheet `0`. Selection visibility (`CanvasModel::get_show_selection`,
default `true`) is captured too but is infallible by design, so it can
never itself hold an attempt — it exists so a deliberately
selection-less host (`show_selection(false)`) is distinguishable from a
genuine bridge failure on `get_selected_view()`.

Any one of the fallible reads failing holds the *entire* taken
`PendingWork` — merged back into `self.pending` verbatim — before
`Chrome::classify`, `plan_frame`, any `Chrome` mutation, cache
invalidation, paint, or presentation runs: neither surface presents, no
painter operation is emitted, and `last_frame` plus decoration snapshots
are left untouched. The attempt is stamped
`FrameOutcome::HeldOnInputFailure(FrameInputFailure)` — naming which
read failed (`SelectedSheet`, `SelectedView`, `SheetMismatch`,
`FrozenRows`, `FrozenColumns`, `RowHeaderVisibility`,
`ColumnHeaderVisibility`) — and `paint_if_dirty` returns
`PaintResult::Retry`. Before this existed, a failed scalar read fell
back to a synthetic default (sheet `0`, an all-A1 view) and painted
anyway, against fabricated state; capture makes that failure observable
instead.

The scroll fast path is a second typed verdict living inside
`FrameDelta` itself (`Scroll(BlitPlan)`, a geometric diff against the
committed frame — no signal bit needed) rather than a separate
function.

---

## 2. Canvases and the back buffer

Two stacked `<canvas>` elements, one `WebSurface` each:

```
overlay canvas   alpha:true, desynchronized:true — draws DIRECT
grid canvas      alpha:false (opaque)            — DOUBLE-BUFFERED
   └── detached back <canvas> (not OffscreenCanvas — keeps the ctx type
       CanvasRenderingContext2d so CanvasPainter needs no second plumbing)
```

The grid painter draws into the detached back canvas; `present()` does a
single 1:1 `drawImage(back, 0, 0)` onto the visible front canvas (image
smoothing disabled so the copy never resamples). Consequences:

- Partial repaints (Damage, SlotsReuse, blit) are **cumulative in the
  back buffer** — prior pixels persist there, so a band repaint only has
  to paint the band. The front canvas always receives a complete frame.
- The user never sees an intermediate state (cleared band without new
  text); tearing-class bugs become impossible rather than unlikely.
- Every paint arm calls `present()` once per layer it actually painted —
  the "painted ⇒ present" wiring is per-arm and explicit.

The overlay has no back buffer: it is cleared and fully redrawn every
time it paints (it's a handful of rects/strokes — redrawing is cheaper
than tracking damage for it).

Scroll blits (`BlitPainter::blit` → `ctx.drawImage(front_canvas, …)`)
also read the *front* canvas as the source of kept-band pixels.

---

## 3. What rerenders when you edit a cell (today)

Trace for typing `42` into `B3`, committed without moving the selection
(e.g. via the formula bar's confirm button — pressing Enter also fires
`view_changed()` from the resulting navigation, which forces `Fresh`
instead: content plus view always plans `Fresh`, never a blit or a
band-clipped `Damage`, per `plan_frame`'s table). RustyCalc consumer as
wired now (`src/components/workbook/worksheet/subscribe.rs`):

```
1  IronCalc: set_user_input → recalc
2  RustyCalc event bus: ContentEvent::CellChanged { address, .. }
   (+ CalculationUpdated { affected_sheets } if dependents recalced)
3  subscribe effect: has_content → ic.mark_content_dirty()   ← UN-ROWED
4  Engine: PendingWork::mark_panes(PaneRegionMask::ALL) →
           content = ContentWork::Panes(ALL)
5  rAF → paint_if_dirty:
     FrameInputs::capture — sheet/view/freeze/headers all read clean
     Chrome::classify → Stable (no geometry or scroll change)
     plan_frame(work, Stable, ..): content is Panes(ALL), not Rows, so
     Damage's sheet-match arm never applies (no Rows to match) →
     SlotsReuse / GridWork::Panes(ALL)
6  paint_slots_reuse_regime:
     PaneCache::invalidate(ALL) + invalidate_paint_cache()
     Chrome::next reuses prev slot vecs (no geometry walk)
     for each visible pane (up to 4 with freezes), render_pane:
       bulk refetch: styles, values, cell_types, decorations   ← 4 JS calls/pane
       scratch = rebuild_pane_fingerprint_in_place(…) into PaneCache's
                 warm scratch tree (pane → row → cell digests)
       scratch.digest == painted.digest (same range)?  → SKIP the paint
                 walk (pixels stay); commit scratch → painted anyway
       differ → plan_pane_repaint(painted, scratch) decides:
                  Rows(spans) → clear + repaint only those row bands
                  Full        → clear the whole pane rect, repaint ALL cells
                commit scratch → painted (mem::swap, zero allocation)
7  grid.present()  — one back→front drawImage
8  overlay repaint (active-cell hook repaints B3's grid pixels on the
   overlay so DEL/edit is never shown stale under the cursor)
```

So today: **data is refetched for every visible pane, but pixels repaint
only in the row bands whose fingerprint actually changed** — typically
one row in one pane. The compare step the question asks for ("check the
IronCalc data, then decide") already exists; it lives at *row*
granularity within each pane, in the fingerprint tree.

### The three tiers of damage detection

The engine deliberately layers three mechanisms, coarse-to-fine:

1. **Caller hints** (`mark_content_dirty(mask)` / `mark_rows_damaged`):
   cheap, set by whoever knows what changed. Trusted for *routing*
   (which regime, which panes to refetch) but never for correctness.
2. **Pane fingerprint tree** (`PaneCache`'s `PaneFingerprintState`, in
   `renderer/cache/pane_cache.rs` + `renderer/cell/fingerprint.rs`): a
   pane → row → cell digest tree, not a single scalar. The whole-pane
   digest is the safety net — a content change nobody marked (upstream
   recalc a caller forgot) is still caught here, because on slots-reuse
   frames the bulk fetch is unconditional and the fingerprint compare
   decides the paint. On a mismatch, `plan_pane_repaint` walks the two
   trees row-for-row and narrows the repaint to just the changed row
   bands (merged, capped at `MAX_DAMAGE_SPANS` via the same
   `ContentWork::normalize_rows` logic the Damage regime uses) — unless any
   changed span's internal top/bottom boundary carries explicit-border
   risk in either tree, in which case it falls back to a whole-pane
   repaint (see below). Hints make things *fast*; the fingerprint tree
   keeps things *correct*, and the row-damage plan makes "correct" also
   *cheap* within a changed pane.
3. **Strip repaint** (`splice_strip_into` + the shared `paint_cells_pass`
   tail, called from `execute_blit_pane` and `execute_damage_pane`): the
   surgical tool. Fetches only a band, splices it into the cached pane
   buffers, and clears + repaints only that band — it never
   commits into the pane's painted-pixel tree (a partial buffer can't
   stand in for a whole-pane hash), so the tree stays whatever the last
   full/row-band paint left it until a later frame's comparison naturally
   accounts for the drift (see §4). Shared by the blit path (revealed
   strip) and the Damage path (edited rows) — and structurally distinct
   from the row-band repaint in tier 2, which reads the buffers
   `render_pane` already bulk-fetched this same call rather than fetching
   a strip.

The `StyleDigest` field list in `fingerprint.rs` is load-bearing in both
directions: a paint-read field it misses → stale pixels on skip; a
paint-irrelevant field it includes → wasted repaints. That contract is
written at the type, which is where the next maintainer will look.

### Why the repaint unit is a full-width row band, not a cell

Two independent reasons, both documented on `ContentWork` itself (the
right home: the constraint explains the type's shape):

- Cell text paints last and **unclipped** — it may overflow horizontally
  into row neighbours. A per-cell repaint could erase a neighbour's
  overflow or orphan its own.
- A row's border may be owned by (painted from) the row *above*/*below*
  it — and not only via an explicit top/bottom border on that shared
  edge: a `Medium`/`Thick` *left* or *right* border's stroke extends
  `width_px / 2` past its own cell's top and bottom edges too, to close
  the perpendicular corner gap cleanly (`paint_border` in
  `renderer/cell/borders.rs`). A repaint clipped to just the changed row
  could leave a stale shared-edge stroke behind, or fail to draw a newly
  added one, on the pixel row it shares with an untouched neighbour —
  regardless of which edge the border was actually declared on.
  `plan_pane_repaint`'s border-safety check (`fingerprint.rs`) exists for
  exactly this: at each changed span's internal boundary it inspects both
  the `painted` and `scratch` trees' `has_any_explicit_border` flag (true
  when ANY cell in the row carries an explicit border on any of its four
  edges) on the span and its neighbour, and falls back to a whole-pane
  repaint whenever either tree shows risk there — whether the border is
  old (about to be erased), new (about to be drawn), or simply unchanged
  on a neighbour the narrow repaint would otherwise never touch. The
  check is deliberately direction-agnostic rather than top/bottom-only,
  since the corner-extension bleed above means a purely left/right
  border can reach across the row boundary too.

A full-width row band is the smallest repaint unit that cannot go wrong
either way.

### The wired-but-unfed fast path

`markRowsDamaged(sheet, r1, r2)` exists on the wasm facade and the
whole engine path behind it works (`ContentWork::Rows` → `Damage` regime
→ `render_pane_damage` → band strips). **RustyCalc does call it** —
`subscribe.rs`'s content-event match routes `ContentEvent::CellChanged`
and `RangeChanged` through `mark_rows_damaged`. In the common case the
win doesn't land, though:
a cell edit's batch almost always also carries
`ContentEvent::CalculationUpdated` (recalculated dependents), which is
un-rowed and calls `mark_content_dirty()` — collapsing the queued
content work to `ContentWork::Panes(ALL)` (row precision, once lost to
an unscoped raise, never comes back within one attempt) and landing the
paint in `SlotsReuse { ALL }` instead of `Damage`.

What wiring it would take, and what it would buy:

- `ContentEvent::CellChanged`/`RangeChanged` already carry the address —
  those edits can name their rows today. One edited cell → refetch and
  repaint **one row band** instead of bulk-refetching four panes.
- `ContentEvent::CalculationUpdated` only carries `affected_sheets`.
  Dependent-cell recalcs therefore can't name rows until the IronCalc
  bridge surfaces a changed-cells diff. Until then, mixed batches
  correctly degrade: one un-rowed raise poisons the whole batch to the
  `SlotsReuse` path — conservative, never wrong.
- Degradation is built into `ContentWork::merge`: >8 disjoint bands
  (`MAX_DAMAGE_SPANS`), a second sheet, or any un-rowed raise all
  collapse to `Panes(ALL)` → pane-mask path. The fine path can only ever
  *win*; it can never paint less than correctness requires.

---

## 4. Cache inventory

All owned by `RendererCore`; three lifetimes, each with one invalidation
story:

| Cache | Lifetime | Invalidation |
|---|---|---|
| `FrameCache` (text slots, wrap buf, strip-fetch scratch) | one paint call | wiped/refilled every walk — nothing to invalidate |
| `PaneCache::PaneBuffers` (styles/values/types/decorations + last range, per pane) | cross-frame, renderer lifetime | `PaneCache::invalidate(mask)` drops the cached buffer `range` only, on content change; `PaneBuffers::apply_shift` (called from `execute_blit`, after the strip fetch is already confirmed clean) rotates the buffers in place on blit; a range mismatch self-detects (`render_pane_damage` demotes to full walk) |
| Intern tables (`FontIntern`, `ColNameIntern`, `ColorIntern`) | renderer lifetime | never — pure dedup, content-addressed |
| `PaneFingerprintState` (per-pane `painted`/`scratch` `PaneFingerprint` tree pair, inside `PaneCache::PaneBuffers`) | cross-frame, renderer lifetime — **not** on `Chrome` | No separate invalidation marker: each tree's own `range` is folded into its `digest`, so a stale `painted` tree can only coincidentally digest-match a freshly rebuilt `scratch` tree if the range and full content are genuinely identical — in which case the on-screen pixels really are still correct, so `Skip` is the right answer regardless (see below). A successful skip or paint commits `scratch` → `painted` via `mem::swap` (zero allocation, zero clone); a strip paint does not commit — `painted` is left exactly as the last full/row-band paint left it |
| `CanvasPainter` `SetterCache` (last fill/stroke/font/line-width) | painter lifetime | `invalidate_cache()` on theme change + DPR change |

**Pane cache metadata never describes speculative state.** The renderer
returns one owned `PreparedCacheCommit` containing only panes that actually
executed. `Orchestrator::finish_attempt` applies it once through
`RendererCore::install_pane_cache_commit`, before publishing/presenting the
matching frame. A prepare step that hits a bridge failure touches only
renderer-lifetime scratch; abort recycling returns every earlier prepared
bundle and candidate tree to its originating scratch pool. Held therefore
installs nothing, while Partial installs only healthy panes.

**One staleness axis, not two.** `PaneCache::invalidate` (buffer range)
says "the pane's *data* may be stale, refetch it" — a content-dirty
`SlotsReuse` signal calls this, and dropping just the buffer range lets a
refetch that comes back byte-identical to what's already on screen (e.g.
a dirty signal with no real edit behind it) still `Skip` via the intact
`painted` tree, instead of forcing an unconditional repaint. There used
to be a second, actively-maintained "the pane's *pixels* may be stale"
marker on `PaneFingerprintState`; it's gone, because it was provably
redundant with the tree's own range-in-digest property above — a stale
tree self-disqualifies on comparison, so no separate flag was needed. A
*rejected* strip fetch (a transient `BridgeFailed` on any of its four
buffers, or a blit-frame preflight failure before any pixel shifts)
touches neither the buffer range nor the painted tree — the relevant
preflight is atomic, so on rejection the pane's cached buffers, on-screen
pixels, buffer range, and painted tree are all left exactly as they were.

Cross-cutting rule visible in the table: **whoever writes a cache owns
its invalidation**, and stale-tolerant caches (interns, scratch) are
kept structurally incapable of going stale rather than needing
discipline. The one cross-object dependency — the painted-pixel tree
must agree with what's actually on screen — is maintained passively: a
strip paint never commits into `painted`, so `painted` only ever
reflects a full or row-band paint's actual output, and any drift a strip
splice introduces is caught by the range-in-digest property the next
time a full pane comparison runs.

`Fetched<T>` (`Value | Absent | BridgeFailed`) flows through all bulk
buffers so a per-cell JS bridge failure is distinguishable from a
legitimately blank cell: on slots-reuse frames a `BridgeFailed` fetch
holds the previous pane atomically (old buffers parked back, no clear,
no fingerprint commit) instead of flashing blank.

---

## 5. Blit (viewport shift) — the short version

`Chrome::classify` detects the shift geometrically — comparing the
captured view's effective scroll origin against the committed frame's,
no signal bit involved — and re-hashes the active cell before trusting
the fast path: a missing or mismatched active-cell snapshot rejects it
outright (`Rebuild(MissingActiveSnapshot)` /
`Rebuild(ActiveCellChangedOrUnknown)`), and a two-axis move is never a
single blit (`Rebuild(TwoAxisScroll)`). Producing
`FrameDelta::Scroll(plan)` is necessary but not sufficient: `plan_frame`
only selects it when the attempt carries no content or geometry work —
blitting stale pixels over changed values is the recalc-bug class this
guards against, and since every current geometry producer already
forces a `Chrome::classify` hard break, this is a defensive second gate
for a future producer that doesn't happen to trip one.

Once selected, the live orchestrator path is `Chrome::prepare_blit`,
which builds the same in-place candidate `Chrome::next_blit` does (there
is only one blit construction algorithm) but holds it open as a
`PreparedBlitFrame` rather than committing immediately: `Ok(prepared)`
is the shift-kept-band case (strip-paint the reveal), `Err(prev)` is the
`FreshFallback` demotion — e.g. the row-header digit boundary widening
past what the kept band assumed. `paint_viewport_regime` runs the strip
prefetch *after* `prepare_blit` returns, against the candidate it
already built; on a bridge failure there it calls `prepared.rollback()`,
which moves `prev`'s untouched slot/header vectors back out of the
discarded candidate — no `Chrome::clone` — reconstructing exactly the
`Chrome` that was committed before the attempt. `Chrome::next_blit`
itself stays public — used by direct geometry tests, returning the same
two-variant `BlitOutcome` (`Blitted` / `FreshFallback`) and committing
through the identical builder immediately, with no held-open candidate.
Per pane, `PaneCache::classify_shift` returns `PaneShiftPrep::{Shifted,
MissingCache, IncompatibleRange}` — the reason a pane can't blit is a
named variant, and the fallback is always the ordinary full-pane walk.

---

## 6. How to model this kind of code — the rules that keep it maintainable

These are the patterns the codebase already follows and that any new
optimization should follow. They are what makes a five-regime pipeline
reviewable by a human.

**1. Disjoint change classes get disjoint inputs, never one dirty bit.**
Geometry rebuild (`GeometryWork`), view movement (`view: bool`), content
change (`ContentWork` — named rows or a pane mask, one sum type so row
precision and whole-pane precision share a single field instead of two),
and overlay state (`overlay: bool`) are four separate fields on one
`PendingWork` value. Viewport shift itself is not a fifth stored input —
it is *derived* geometrically from view movement by `Chrome::classify`,
never a bit of its own. The historical bug class here was dispatching
blit and content through one flag. If two kinds of change need different
repaint strategies, they must arrive as different data.

**2. Decisions are values; effects live in named arms.**
`FrameDelta`, `GridWork`, `OverlayWork`, `BlitOutcome`, `PaneShiftPrep`,
`ContentWork` — every branch point returns a `#[must_use]` enum whose
variants *name the outcome*, then an exhaustive `match` runs exactly one
arm. Nobody has to reconstruct "what will happen" from boolean soup;
the compiler polices completeness when a variant is added. When you add
an optimization, add a variant, not a flag.

**3. Hints route, data decides.**
Caller-supplied dirt (`mark_*`) chooses the cheap path; a content
compare (fingerprint) makes the final paint/skip call against actually
fetched data. This means a missing hint costs performance, never
correctness — the only sustainable contract when hint call sites live
in another crate (or another language).

**4. Every fast path names its own escape hatch.**
Damage falls back to SlotsReuse (`ContentWork` collapsing to
`Panes(ALL)`), blit falls back to Fresh (`FreshFallback`), a mismatched
pane range falls back to the full pane walk. Fallback is the ordinary
slow path — never a special recovery mode — and the *reason* for
falling back is an enum variant (or, for `ContentWork`, a merge-table
outcome) you can log, test, and grep.

**5. Invariants live on the type or function that owns them.**
"Full-width bands because text overflows, and because rows can share
border ownership" is documented on `ContentWork`. "A successful strip
paint never touches the painted-pixel tree" is enforced inside
`install_pane_cache_commit`'s `PaneCacheCommit::Splice` arm. The
row-damage planner's border-safety rule sits on
`plan_pane_repaint`/`span_has_unsafe_border`. The fingerprint's
hash-domain contract sits on `StyleDigest`. When the constraint and the
code that must respect it are in one place, the optimization survives
its author.

**6. Correctness invariants get structural enforcement, not comments.**
Back buffer makes partial repaints tear-free by construction;
`Fetched::BridgeFailed` makes bridge failures unrepresentable as blank
cells; pass order (bg → CF → grid borders → explicit borders → text)
is centralized in one `paint_cells_pass` shared by all four entry
points (full pane, blit strip, damage band, row-band span) so it
cannot drift between paths.

**7. One new capability = one new seam, reusing the existing machinery.**
The Damage regime added *no* new paint machinery — it reuses the blit
path's strip fetch/splice/paint: `splice_strip_into` and the shared
`paint_cells_pass` tail. If a proposed optimization needs a parallel copy
of an existing pass, the design is wrong; find the seam (here:
`splice_strip_into` / `paint_cells_pass`) and feed it a different range.

**8. Observability is part of the pipeline.**
`last_regime`/`last_work_flags` stamp every paint; the recorder captures
per-frame op logs attributed to a regime. An optimization you cannot
attribute frames to is one you cannot verify or bisect.

---

## 7. Current gaps (ranked by value)

1. **Wire `markRowsDamaged` from RustyCalc** for `CellChanged` /
   `RangeChanged` events (addresses already available). Single-cell
   edits drop from 4-pane bulk refetch + 1-pane repaint to 1 row-band
   fetch + repaint. `CalculationUpdated` keeps the un-rowed path until
   the bridge exposes a changed-cells diff — mixed batches degrade
   correctly by design.
2. **Changed-cells diff from IronCalc** (`CalculationUpdated` carrying
   rows, or a diff query on the bridge). Unlocks the Damage path for
   formula-dependent updates — the common case in a live spreadsheet.
3. Column-clipped damage is intentionally **not** a gap: unclipped text
   overflow and cross-row border ownership both make full-width bands the
   correct minimum (§3, "Why the repaint unit is a full-width row band,
   not a cell"). Revisit only if overflow ever becomes clipped/measured
   per cell.
