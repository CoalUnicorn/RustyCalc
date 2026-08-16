# Granular Live Frame Diagnostics

**Date:** 2026-08-16

**Status:** Draft

**Scope:** dev-only inspection of live `iron-canvas` frame planning, grid layout,
fetching, repaint selection, and cache transitions

**Verified against:** RustyCalc `56b249e`

## Purpose

The current one-line `frameTrace()` is useful for spotting a suspicious frame,
but it cannot explain that frame. This design adds an optional, structured
drill-down for development builds so a developer can answer:

1. Which visible address segments were planned?
2. Which ranges were fetched, and why?
3. Why did the grid choose `skip`, `rows`, `FULL`, or `strip`?
4. What cache action was prepared and committed?
5. For a blit, which source, destination, clip, and revealed strip were used?

This is a companion to
`2026-08-11-render-debugging-and-maintainability.md`. That design owns the
compact trace and recorder timeline. This document focuses on interactive,
live diagnosis and does not reopen the single-grid cache model.

## Browser observations to explain

### Freeze toggle at B3

| Action | Trace | Addressed cells |
| --- | --- | ---: |
| Activate freeze | `Fresh[GEOMETRY | OVERLAY] grid:FULL fetched=1856` | 464 |
| Deactivate freeze | `Fresh[GEOMETRY | OVERLAY] grid:FULL fetched=1920` | 480 |

`fetched` is the number of logical channel slots, not the number of addressed
cells. The current fetched bundle has four channels, so the two observations
represent 464 and 480 addressed cells. The 16-cell difference cannot be
attributed further from the string alone.

A freeze change is expected to force `Fresh`: `Freeze` is one of the closed
`RebuildReason` variants. The missing facts are the before/after `GridShape`,
the exact TL/TR/BL/BR segment ranges, and the fetch ranges derived from them.

### Isolated quadrant edits

| Intended quadrant | Trace | Addressed cells |
| --- | --- | ---: |
| TL | `SlotsReuse[VIEW | CONTENT | OVERLAY] grid:skip fetched=1508` | 377 |
| TR | `SlotsReuse[VIEW | CONTENT | OVERLAY] grid:rows1/1 fetched=1508` | 377 |
| BL | `SlotsReuse[VIEW | CONTENT | OVERLAY] grid:skip fetched=1508` | 377 |
| BR | `SlotsReuse[VIEW | CONTENT | OVERLAY] grid:skip fetched=1508` | 377 |

All four attempts fetched the same 377 addressed cells. Only TR produced one
changed row span containing one row. If TL, BL, and BR were genuine visible,
paint-relevant value changes, `grid:skip` needs an explanation before these
runs can be used as Task 4 cost evidence.

The diagnostics should distinguish at least these cases:

- the edited address was outside the planned visible segments;
- the final paint inputs were unchanged, for example an edit to the same value;
- the model revision or event did not correspond to the intended edit;
- the affected row fingerprint compared equal;
- the manual quadrant label did not match the actual freeze/view geometry.

The design must not restore four independent pane verdicts. There is one grid
fingerprint and one grid verdict. Segment and range metadata are attribution
for that verdict, not new control-plane state.

## Byte-for-byte comparison

This should be an automated browser-test assertion rather than a manual browser
procedure.

1. Render the retained path on one Canvas2D surface.
2. Build an independent canvas in the same final model, viewport, freeze, DPR,
   theme, and header state.
3. Force that second canvas through `Fresh`.
4. Read both grid surfaces with `getImageData`.
5. Compare the complete RGBA byte vectors.

`iron-canvas-web/tests/render_wasm.rs` already has
`stage6_assert_matches_forced_fresh` and related helpers implementing this
pattern. A failure should report a concise count and a few pixel coordinates;
it should not print multi-megabyte byte arrays. Screenshots and painter
operation equality are useful clues, but neither is a raster-equivalence
oracle.

## Design constraints

- Production builds without `dev-tools` retain no detailed diagnostic state or
  WebAssembly API.
- Optimized diagnostic builds remain possible. `cfg(debug_assertions)` is not
  suitable because Task 4 timing probes need `--release --features dev-tools`.
- The existing `FrameTrace` remains compact, copyable, allocation-free, and
  suitable for the one-line Perf panel summary.
- Detailed capture is runtime-disabled by default so its allocations and clock
  reads do not contaminate measurements unless explicitly requested.
- Diagnostics observe the existing capture, plan, prepare, execute, and commit
  decisions. They do not re-run classifiers or become a second planner.
- No cell values, formatted text, formulas, or fingerprint hashes are exposed.
- A held attempt must not present candidate layout or cache state as committed.

## Proposed interface

### Feature boundary

Keep the public build switch as the existing `dev-tools` feature. If core needs
a narrower switch, add an internal `dev-diagnostics` feature enabled by
`iron-canvas-web/dev-tools`.

### Runtime boundary

Expose two live methods under `dev-tools`:

```text
setFrameDiagnosticsEnabled(enabled: bool)
frameDiagnostics() -> JsValue | undefined
```

`frameDiagnostics()` returns the last completed live attempt. It returns
`undefined` when capture is disabled or during playback. Playback continues to
use `recordingCurrentAttempt()` because it describes the displayed recording,
not the last live frame.

The first version retains only the last snapshot. A ring buffer duplicates the
recorder's timeline role and is not required for manual Task 4 probes.

### Perf panel

Keep the current one-line trace visible. When detailed capture is enabled, the
same panel may provide an expandable structured view and a “copy JSON” action.
No new application panel is needed.

## Diagnostic snapshot

The wire shape should be typed and versioned locally. Core facts can use Rust
enums and structs; the web crate owns their serde projection.

### Attempt summary

- attempt and optional committed sequence;
- selected and effective strategy;
- pending `WorkFlags`;
- outcome and painted layers;
- `FrameDelta` and `RebuildReason`, using the reason already stored in
  `FramePlan` rather than classifying again.

### Geometry and segment layout

- backing width and height, CSS width and height if known by the web facade,
  and DPR;
- sheet, top row, left column, frozen row count, and frozen column count;
- `GridShape` row and column lengths;
- each populated `GridSegment`, in canonical TL/TR/BL/BR order, with its
  `RCRange` and addressed-cell count.

This data should make the 464-versus-480 freeze result directly explainable.

### Fetch accounting

Retain the existing totals:

- renderer bundle requests (`fetch_batches`);
- distinct addressed cells (`fetched_cells`);
- logical channel slots (`fetched_cell_slots`).

Add one entry per renderer-owned fetch with:

- purpose: full segment, damage strip, or blit-revealed strip;
- associated region when one exists;
- exact `RCRange`;
- addressed cells and logical slots.

These are renderer requests, not claims about host or engine call counts. An
adapter may satisfy one bundle request with multiple scalar reads.

### Repaint decision

Record the final `GridVerdict`, exact normalized row spans, and a diagnostic
reason. The reason should name the branch already taken by the fingerprint
comparison, for example:

- `NoPaintedHistory`;
- `LayoutMismatch`;
- `RowAddressMismatch`;
- `SpanCapExceeded`;
- `BorderSafety`;
- `FingerprintsEqual`;
- `ChangedRows`.

For the quadrant observations, this would show whether TL/BL/BR were skipped
because all row fingerprints compared equal, while TR identified one exact row
and segment.

### Cache transition

Capture both the planned transition and the state after completion:

- prior committed layout and fingerprint truth state;
- prepared action: none, replace, splice, shift, or reset;
- fingerprint action: install, mark stale, or reset;
- whether the action committed, was discarded, or was held for retry;
- resulting committed layout and fingerprint truth state.

Only `finish_attempt` should publish the completed snapshot. If preparation or
execution holds, candidate data may be shown under a clearly named `candidate`
field while `committedAfter` remains equal to `committedBefore`.

### Blit detail

For `Viewport`, include:

- axis and logical row/column delta;
- source and destination pixel rectangles;
- effective grid clip rectangle;
- revealed address range and pixel strip;
- preflight result and exact fallback reason.

This is required for diagnosing deep-scroll and frozen-band reuse. A generic
`unshift(range)` label is insufficient to verify clip boundaries.

### Paint counts and timing

The first version should count painted grid rows and addressed cells. Primitive
counts such as text, fill, stroke, clip, and blit may be derived from a
dev-tools `Painter` decorator later if they answer a measured question.

Wall time belongs at the host boundary, around `paintIfDirty`, using the
browser clock. It must be labelled as host-observed duration and kept outside
deterministic core traces. Timing runs should normally leave detailed capture
off, then enable it only for a representative frame requiring explanation.

## Example wire projection

```json
{
  "schemaVersion": 1,
  "attemptSeq": 42,
  "committedSeq": 39,
  "selected": "slotsReuse",
  "effective": "slotsReuse",
  "work": ["view", "content", "overlay"],
  "delta": "stable",
  "rebuildReason": null,
  "geometry": {
    "topRow": 3,
    "leftColumn": 2,
    "frozenRows": 2,
    "frozenColumns": 1,
    "segments": [
      { "region": "tl", "range": { "r1": 1, "c1": 1, "r2": 2, "c2": 1 }, "cells": 2 },
      { "region": "tr", "range": { "r1": 1, "c1": 2, "r2": 2, "c2": 12 }, "cells": 22 },
      { "region": "bl", "range": { "r1": 3, "c1": 1, "r2": 34, "c2": 1 }, "cells": 32 },
      { "region": "br", "range": { "r1": 3, "c1": 2, "r2": 34, "c2": 12 }, "cells": 352 }
    ]
  },
  "fetch": {
    "batches": 4,
    "addressedCells": 408,
    "logicalSlots": 1632,
    "requests": []
  },
  "repaint": {
    "verdict": { "rows": { "spans": 1, "rows": 1 } },
    "reason": "changedRows",
    "changedRows": [{ "r1": 8, "r2": 8 }]
  }
}
```

The numbers above illustrate the shape only. Real diagnostics must report the
renderer-owned values without reconstructing them in JavaScript. Numeric row
and column fields are the wire authority; the UI may add A1-style labels.

## Options considered

| Option | Decision | Reason |
| --- | --- | --- |
| Add more tokens to `frameTrace()` | Reject | Becomes unstable, hard to parse, and still cannot represent ranges safely |
| Compile under `debug_assertions` | Reject | Excludes optimized dev-tools timing runs |
| Structured last-attempt snapshot under `dev-tools` | Adopt | Bounded scope, typed data, zero production-build cost |
| Add the detail to every `.icr` attempt now | Defer | Changes a durable schema and increases every recording before live probes prove the useful fields |
| Log every cell or hash to the console | Reject | High volume, privacy risk, and unstable hash values |
| Restore per-pane cache/verdict state | Reject | Conflicts with the single-grid ownership model; segment attribution is sufficient |

## Expected future plan scope

An implementation plan should split the work so each slice is independently
verifiable:

1. Define the dev-only diagnostic domain types and runtime enable switch.
2. Surface `FramePlan.rebuild_reason` and exact `GridLayout` segments.
3. Instrument renderer-owned fetch requests and repaint-decision reasons.
4. Instrument prepared/committed cache transitions and blit geometry.
5. Add the web wire projection and Perf panel expansion.
6. Add focused native and browser tests, then run Task 4 probes with capture
   disabled for timing and enabled for attribution.

Likely files include core orchestration and renderer modules, core and web
feature definitions, the web facade/wire projection, the rAF perf state, and
the existing Perf panel. The plan should name exact files after another live
source pass because renderer modules may move during the remaining
single-region cleanup.

## Verification requirements

### Native

- Freeze rebuild exposes `RebuildReason::Freeze` and exact before/after
  segments.
- Fingerprint equality reports `skip` plus `FingerprintsEqual`.
- One changed row reports its exact span and segment attribution.
- Every `FULL` promotion reason is covered, including border safety and the
  span cap.
- Fetch request totals equal the sum of their ranges and channel slots.
- Held execution preserves the previous committed layout/cache in the
  published snapshot.
- Diagnostics disabled does not retain request vectors or perform clock reads.

### Browser

- The dev-tools-only API is absent from a production-feature build.
- The B3 freeze toggle explains the segment and fetched-cell difference.
- One isolated edit in each real segment reports the intended address inside
  that segment. Any `skip` must carry an explicit reason.
- Deep vertical and horizontal blits expose exact clips and revealed strips.
- Task 4 retained-pixel scenarios still compare raw Canvas2D `ImageData`
  against independent forced-Fresh output.

### Measurement discipline

For each Task 4 cost probe, record:

- the exact edited address and old/new value class;
- viewport, freeze, DPR, and canvas size;
- one-line trace;
- structured diagnostic snapshot from a representative run;
- fetch batches, addressed cells, logical slots, painted rows/cells, and host
  wall time;
- whether detailed capture was enabled during the timed sample.

Do not describe a `grid:skip` run as the cost of painting a quadrant until the
snapshot proves the intended visible, paint-relevant change reached that
segment.

## Non-goals

- Changing planner eligibility or raster behavior.
- Introducing pane-local caches, fingerprints, or commit semantics.
- Capturing model cell contents.
- Making detailed diagnostics part of the stable production API.
- Replacing automated forced-Fresh `ImageData` comparisons with manual visual
  inspection.
- Expanding the recorder schema before live diagnostic fields prove useful.

## Questions to settle when preparing the plan

1. Should the first UI expose the full JSON only, or render geometry, fetch,
   repaint, and cache sections?
2. Is painted row/cell counting already available at a clean renderer boundary,
   or should it wait for a separate `Painter` counting decorator?
3. Which cache truth states need public diagnostic names without exposing
   internal buffer ownership?
4. Should a later recorder version embed this schema, or store only a smaller
   projection of fields proven useful by Task 4?

Recommended defaults are: last snapshot only, runtime-disabled, JSON plus a
small expandable view, row/cell counts before primitive-operation counts, and
no recorder schema change in the first slice.
