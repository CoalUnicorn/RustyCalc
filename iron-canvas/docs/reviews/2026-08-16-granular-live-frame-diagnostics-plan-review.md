# Review: Granular Live Frame Diagnostics Implementation Plan

**Date:** 2026-08-16

**Reviewed:** `docs/plans/2026-08-16-granular-live-frame-diagnostics.md`
against `docs/designs/2026-08-16-granular-live-frame-diagnostics.md` and live
RustyCalc source at `56b249e`

## Summary

The plan has a sound outer boundary: detailed capture is feature-gated and
runtime-disabled, `FrameTrace` remains compact, core does not read a clock,
the recorder schema stays untouched, and publication belongs at
`finish_attempt`.

It is not implementation-ready. Four blockers prevent the proposed work from
meeting or compiling against the current system: the snapshot cannot connect
an edit to a TL/TR/BL/BR segment, the completion hook misclassifies committed
overlay attempts and uses a moved value, the browser fixtures and wire mirrors
do not match live types, and the Perf-panel toggle conflicts with the
demand-driven rAF and existing CSS layout. Several smaller gaps would also make
the diagnostics incomplete or misleading.

Implementation status: **not implemented**. No `dev-diagnostics` feature,
`renderer/diag.rs`, `FrameDiagnostics`, or live diagnostic wasm methods exist
outside the plan.

## Design Spec Adherence

- Spec: `iron-canvas/docs/designs/2026-08-16-granular-live-frame-diagnostics.md`
- Implementation: N/A; this is a pre-implementation plan audit.
- Matches the design: partial.
- Overall verdict: **revise before implementation**.

The plan preserves the design's production-cost, privacy, transactional,
recorder, timing, and raster-verification constraints. It does not yet deliver
the promised segment attribution, complete grid verdict, exact blit clip, or a
working runtime control path.

## Findings

### 1. Blocker — changed row spans cannot attribute an edit to a quadrant

The proposed repaint section contains only a grid verdict, a fingerprint
reason, and absolute row spans (`plan:1361-1382`). Task 7 then claims to prove
segment attribution by checking whether a changed row intersects the intended
segment (`plan:3211-3233`). That check is ambiguous across the column split:
one changed frozen row intersects both TL and TR, while one changed scroll row
intersects both BL and BR.

This is inherent in the live fingerprint shape. `GridFingerprint` stores one
digest per absolute row, and each digest folds every dense column segment for
that row (`renderer/cell/fingerprint.rs:1-6,29-34`). A `Skip` has no changed
row at all. The proposed snapshot also has no edit/probe address, despite the
manual acceptance step saying its JSON will show “the edited address inside a
segment” (`plan:2980-2982`).

The Task 7 test validates its own hard-coded `(row, col)` against a static
layout before painting. It never proves that this address was the change the
renderer observed. For identical-value edits, the test even accepts either
`Skip` or `Rows` (`plan:3192-3209`), so it cannot resolve the original
TL/BL/BR `grid:skip` question.

**Required revision:** add a diagnostic-only evidence path connecting the
expected change to the attempt. Two coherent options are:

- capture an optional host-supplied probe `RCRange` and report which exact
  segments contain/intersect it; or
- compute diagnostic-only per-segment comparison facts while building the
  candidate fingerprint.

The first option is smaller and preserves the single grid fingerprint. It must
be attempt-scoped and range-only; it must not enter planner eligibility or
carry cell contents. Tests should assert that the captured probe range belongs
to exactly the intended segment and that the attempt's verdict/reason follows
that same probe.

### 2. Blocker — the completion hook confuses cache work with transaction outcome

Task 1 derives `DiagCacheResolution` from `cache_commit.is_some()`
(`plan:640-676`). Those concepts are not equivalent. The live Overlay regime
returns `PaintOutcome::Committed` with `cache_commit: None`
(`orchestrator.rs:1395-1417`). The proposed code would therefore publish a
successful overlay frame as `HeldForRetry` even though it advanced
`committed_seq` and returned `Painted`.

The same snippet computes `overlay_painted` after the current completion code
has consumed `overlay_ctx` with `if let Some(ctx) = overlay_ctx`
(`orchestrator.rs:1302-1337`; proposed insertion at `plan:680-690`). That is a
use-after-move and will not compile.

**Required revision:** derive resolution from the matched `PaintOutcome` or
the resulting `FrameOutcome`/`PaintResult`, never from the presence of a grid
cache commit. Compute the painted-layer facts before consuming the context, or
borrow it with `as_ref()` throughout the common overlay step. Add a focused
overlay-only test asserting:

- `resolution == Committed`;
- `committed_seq == Some(_)`;
- `planned_action == None`;
- grid false and overlay true;
- committed cache before/after unchanged.

### 3. Blocker — the browser tasks are written against fixture and wire shapes that do not exist

Task 7 cannot be implemented mechanically as written:

- `stage6_fixture_store()` returns
  `Rc<RefCell<HashMap<(i32, i32), FixtureCell>>>`; it has no `with_frozen`,
  `frozen_rows`, or `frozen_cols` members. Those belong to
  `StableViewFixture` (`render_wasm.rs:1208-1222,1224-1253`; plan
  `3120-3132,3167-3171`).
- `stage6_canvas_over` performs the cold Fresh paint before returning
  (`render_wasm.rs:1510-1538`). The proposed tests enable diagnostics after
  that return and immediately call `diag_snapshot` (`plan:3123-3128,
  3170-3175`), so the snapshot is still `undefined`.
- `DiagGeometryWire` nests frozen counts under `shape`, but
  `DiagGeometryScenario` expects `frozenRows`/`frozenCols` at the geometry
  root (`plan:2326-2338,3026-3034`).
- `changedRows` serializes `RowSpanWire { r1, r2 }`, but its test mirror is
  `RcRangeScenario { r1, c1, r2, c2 }` (`plan:2445-2468,3044-3066`).
- revealed blit strips serialize only `region` and `range`, but the mirror
  reuses `DiagSegmentScenario`, where `cells` is required
  (`plan:2640-2651,3095-3103`).
- the Task 5 smoke mirror expects `outcome: String`, while
  `FrameOutcomeWire` is internally tagged and serializes as an object with a
  `kind` field (`plan:1993-2000,2160-2187`).

The “identical-value” stimulus also writes the literal `"unchanged"`, while
the fixture starts with `r{row}c{col}` values (`render_wasm.rs:1208-1218`).
That first write is a real change.

**Required revision:** define one reusable test wire mirror that exactly
matches the proposed serialized shape, and test its conversion natively before
browser scenarios. Extend or replace the Stage 6 helper so diagnostics and
freeze controls are installed before the cold Fresh paint. Use the real
existing value for an identical-value case. Remove “adjust to observed
behavior” branches; each test should assert one deterministic property.

### 4. Blocker — the Perf-panel control path does not compile or obey the one-shot scheduler

`use_one_shot_raf` requires `paint: impl Fn() -> bool` and pauses until its
returned `poke` is called (`one_shot_raf.rs:14-37`). Task 6 proposes a mutable
local `diag_enabled_pushed` and assigns to it inside that `Fn` closure
(`plan:2790-2814`). Rust rejects that mutation. The established pattern here
is `Cell<bool>`.

Even after replacing the local with `Cell`, changing the Perf signal does not
wake the paused loop. Enabling happens only because the manual test commits a
cell afterward. Disabling is not pushed immediately, and closing the Perf
panel while the toggle is on leaves detailed capture active. That contradicts
the claimed opt-in lifecycle and can contaminate later timing samples.

Two UI snippets also conflict with live APIs/layout:

- `Navigator::clipboard()` returns `Clipboard`, not `Option<Clipboard>`, so
  `if let Some(clipboard) = ...clipboard()` does not compile
  (`plan:2863-2872`; existing calls in `workbook/mod.rs` use it directly).
- `.pp` has `overflow: hidden` (`styles/panels/perf-panel.css:11-24`), so the
  proposed absolutely positioned child panel below it is clipped
  (`plan:2907-2911,2942-2954`). The parent is also not positioned, leaving the
  absolute containing block unspecified for this use.

**Required revision:** make toggle changes wake the worksheet rAF, use a
`Cell<bool>` for last-pushed state, and force capture off when the panel closes
or the worksheet unmounts. Use the existing `Popover`/portal pattern or another
non-clipped existing UI container for JSON. Follow the current clipboard API
and surface copy failure rather than silently swallowing it.

### 5. Should fix — `NoPaintedHistory` is assigned to rebuilds with painted history

Task 3 assigns `NoPaintedHistory` to every full paint whose `Chrome` did not
reuse slots (`plan:1300-1321`). A freeze, DPR, theme, sheet, header, or size
rebuild can all have an existing painted fingerprint; the comparison simply
was not run. The probe runbook demonstrates the error by labelling a B3 freeze
toggle `FULL, noPaintedHistory` (`plan:3391-3395`).

This makes the new diagnostic confidently state a false cause. It also hides
the useful distinction between “cold start” and “Fresh forced by
`RebuildReason::Freeze`.”

**Recommended fix:** reserve `NoPaintedHistory` for
`FingerprintState::painted.is_none()`. Add a reason such as
`ComparisonNotRun`/`FreshRebuild`, or leave the fingerprint reason absent and
use the already captured `rebuild_reason` as the authority. The latter avoids
duplicating the classifier's reason vocabulary.

### 6. Should fix — Strip verdicts and the effective blit clip are missing

`diag_repaint` is called only in the `PreparedGrid::Full` arm
(`plan:1419-1424`). Damage and Blit update paint counts and fingerprint action
but never populate `repaint.verdict`, even though live execution stamps
`GridVerdict::Strip` for both. The structured snapshot can therefore disagree
with the one-line trace on two of the five strategies.

The design asks for the effective grid clip rectangle. The proposed
`DiagBlit`/wire carries `src`, `dst`, and `strip`, but no `pixel_clip`
(`plan:395-407,1653-1662,2654-2680`). `pixel_strip` is the newly exposed band;
it is not the clip applied around blit strip painting. Live
`PreparedGrid::Blit` already owns the finalized `pixel_clip`, so this fact can
be recorded without re-derivation.

**Recommended fix:** record `GridVerdict::Strip` for Damage and Blit, with an
explicit absence/not-applicable fingerprint comparison reason. Add
`pixel_clip` to the blit domain and wire types, and assert it against the
prepared blit work in native and browser tests.

### 7. Should fix — task ordering makes Task 2's green gate impossible

The Task 2 freeze test asserts that the sum of geometry segment cells equals
`diag.fetch.addressed_cells` (`plan:796-798`). Fetch capture is not introduced
until Task 3 (`plan:1333-1417`). Task 2 nevertheless requires its diagnostic
test and then the full core suite to pass before commit (`plan:958-968`). At
that point `diag.fetch.addressed_cells` is still the default zero.

**Recommended fix:** move the fetch-total assertion to Task 3 or populate
fetch facts in Task 2. Keep each task's stated test gate attainable without
future-task code.

### 8. Nit — the plan's locked metadata already contains source drift

The tech stack says “Leptos 0.19”; the root crate uses Leptos 0.8 and
`leptos-use` 0.19 (`Cargo.toml:11,35`). The plan also says `RCRange` exposes
`rows()`/`cols()` and then supplies a fallback if the name differs
(`plan:1828-1853`); live `RCRange` exposes `height()`, `width()`, `rows()`, and
`columns()`, but no `cols()` (`types/coord.rs:13-30`).

These are small corrections, but they undermine the “File Map locked against
working tree” claim. Replace approximate API names before handing the plan to
an implementation agent.

## Checklist

- [x] Production boundary: detailed state and wasm methods are compiled out
  without `dev-tools`/`dev-diagnostics`.
- [x] Transaction boundary: the intended sole publisher is
  `finish_attempt`, after cache installation.
- [x] Privacy: no values, formulas, formatted text, or hashes enter the
  proposed snapshot.
- [x] Timing boundary: core performs no clock reads; host wall time remains
  separate.
- [x] Recorder boundary: `.icr` v5 stays unchanged.
- [x] Raster gate: forced-Fresh Canvas2D `ImageData` remains the correctness
  oracle.
- [ ] Segment attribution: the attempt has no edit/probe range or
  per-segment difference evidence.
- [ ] Completion truth: committed overlay work is mislabelled as held.
- [ ] Complete grid verdict: Damage and Blit omit `Strip` from the structured
  section.
- [ ] Exact blit geometry: the effective `pixel_clip` is absent.
- [ ] Deterministic test plan: native task ordering and browser fixtures/wire
  mirrors do not currently compile or pass as specified.
- [ ] Dev-tool lifecycle: toggle changes do not reliably wake, disable, or
  unmount-clean the capture state.

## Verification Performed

- Read the complete 3,424-line plan and its companion design.
- Compared proposed types and call sites with live `FrameTrace`,
  `finish_attempt`, Overlay construction, grid fingerprint/repaint planning,
  prepared cache/blit execution, the wasm wire module, Stage 6 fixtures,
  `use_one_shot_raf`, Perf panel, and its CSS.
- Confirmed the feature is not implemented in live source.
- Performed static API and serde-shape checks only. No Rust or browser tests
  were run because the proposed code has not been applied.

## Recommendations

1. Add an attempt-scoped range-only probe/expected-change field or another
   concrete per-segment evidence mechanism. Rewrite the quadrant acceptance
   test around that field.
2. Redesign `publish_diag` inputs around transaction outcome, then add the
   committed Overlay test before any renderer instrumentation.
3. Replace the Task 5/7 wire mirrors and Stage 6 setup with fixtures that
   compile against the declared schema and enable capture before the measured
   attempt.
4. Specify an immediate wake/disable/cleanup path for the Perf toggle and use
   an existing non-clipped popup primitive.
5. Correct Fresh reason semantics, populate Strip verdicts, and include the
   finalized blit clip.
6. Repair task sequencing and current API/version names, then re-run a static
   pass before implementation begins.

## Book-worthiness

The semantic distinction in Finding 2 is a useful Chapter 20 candidate after
implementation: “no cache action” is not “no transaction.” Rust's type system
does not prevent two optional outputs from being treated as equivalent when
the domain says otherwise. The other findings are plan/source drift and are
better kept in this review ledger.
