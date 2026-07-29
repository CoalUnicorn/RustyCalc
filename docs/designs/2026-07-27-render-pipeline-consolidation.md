# Render pipeline consolidation — findings & revision proposal

Status: Rev 2 (2026-07-28) — reconciled with the transactional review
(`iron-canvas/docs/designs/2026-07-27-transactional-render-pipeline.md`),
which is adopted as the architecture baseline; this doc keeps the findings
inventory, adds the verified interaction-model contract, and maps Rev 1's
moves onto the transactional stages. Sources: full source sweep 2026-07-27
(renderer internals, orchestrator+scheduling, test inventory) + input-layer
interaction trace 2026-07-28, branch `canvas-api-cleanup` at `e79416d`.

**Update (2026-07-29):** Stage 0-1 landed, see
`docs/superpowers/plans/2026-07-28-transactional-stage-0-1.md`.

## Problem

The five-regime dispatch (`decide()`) is still clean, but the *decisions*
have migrated downstream. Since the fingerprint work (24488ec, e79416d)
the pipeline contains **two parallel "what changed" systems** with
different fetch and cache-commit semantics, plus 13 renderer-side
re-decision sites, 4 copies of the fetch/validate sequence, and an
implicit cross-call staging protocol. That is the "moving parts all over
the place" feeling. The transactional review's deeper diagnosis: one
render attempt has no single commit boundary, so failure observed inside
the renderer cannot stop presentation, `last_frame` advance, or
pending-work consumption owned by callers.

## Findings

### F1 — Two parallel row-repaint systems (the core sprawl)

| | Damage regime (consumer hint) | SlotsReuse + fingerprint (data diff) |
|---|---|---|
| Row source | `markRowsDamaged` → `CellDamage::Rows` | `plan_pane_repaint` → `RepaintPlan::Rows` |
| Fetch | band cells only | full-pane bulk refetch |
| Paint path | `render_grid_damage` → `render_pane_damage` → `render_pane_strip` | `render_pane` → `paint_pane_row_spans` |
| Fingerprint commit | **never** (strip paths don't commit) | commits `scratch` → `painted` |

Same goal, two mechanisms, two cache stories. Their divergence has a
concrete cost: because strip paints never commit, **after any scroll or
damage paint the next SlotsReuse frame is forced to `Full`** — the
painted tree's range-in-digest self-disqualifies (`fingerprint.rs:483`),
so `RepaintPlan::Rows` is unreachable exactly when it would pay off.

*Rev 2 calibration:* this cliff is real and test-pinned
(`frame_trace_names_the_post_blit_slots_reuse_paint_as_full`) but it is
**not** the measured cost — see "Measured evidence" below.

### F2 — Renderer re-decides after `decide()` (13 sites)

Demotions/holds at `orchestrator.rs:825`, `cell/mod.rs:147,490,498,527,
730,756,820`, `renderer/mod.rs:308,332,345`, `fingerprint.rs:483,510,518`.
Each is individually justified ("hints route, data decides"), but the
regime chosen by `decide()` no longer predicts what actually paints.

### F3 — Duplicated sequences

- 4-accessor fetch + `trace_fetch` + 4× `has_bridge_failure`: **four
  copies** — `cell/mod.rs:137-152`, `:511-519`, `:654-662`, `:814-823`.
- Buffer take/park + `range.set`: `cell/mod.rs:389-393` and `:450-455`.
- `classify_shift → plan_blit_pane → widen_blit_strip_to_pixel_clip`
  written twice (pure preflight `cell/mod.rs:593-612` vs mutating
  `renderer/mod.rs:316-335`) — they must agree by hand.
- Clear-rect + narrowed-pane paint: `paint_strip_from_fetched:887-911`
  vs `paint_pane_row_spans:431-447`.

### F4 — Implicit staging protocol

`RendererCore.blit_stage[4]` (`renderer/mod.rs:151`): `prefetch_blit_strips`
must run before `render_pane_blit` or `:730` silently double-fetches;
`full_pane` stage survives the frame guarded only post-hoc by kind+range
checks (`cell/mod.rs:564-568`). `BlitPaneWork.prev_range/new_range/axis`
are never read; `BlitStripStage.strip` feeds only a `debug_assert`.

### F5 — Orchestrator arm drift

Shared skeleton duplicated 5× with unexplained differences:
- Overlay arm runs `refresh_overlay_state` **before** its `last_frame`
  guard (`orchestrator.rs:792`) — does work even when it early-returns.
- Fresh arm's overlay gate lacks the active-cell clause Damage/SlotsReuse
  have (`:958` vs `:875`/`:925`).
- `render_grid_damage:383` walks `ALL` panes; every other entry walks
  `frame.stale_panes`.

### F6 — Dead weight

- `GridSignals::VIEWPORT`: no raiser exists, but it sits inside
  `GRID_ANY`/`ALL`, silently widening `grid_dirty()`.
- `build_pane_fingerprint` / `diff_changed_cells`: `#[allow(dead_code)]`,
  test-only.
- `FrameOutcome::HeldOnBridgeFailure`/`PaneVerdict::Held` reach only the
  perf panel `Display`; `Orchestrator::last_trace()` has zero test callers.

### F7 — Consumer side (RustyCalc)

- Two independent schedulers: worksheet `raf_loop` + one `use_one_shot_raf`
  **per camera**. `camera/canvas.rs:116-121` comments still describe the
  old unconditional loop.
- `raf_loop.rs:106` polls every frame until canvas construction — the one
  remaining non-demand-driven path.
- `subscribe.rs:95-120` mixed-batch hazard: one un-rowed content event in
  a batch poisons `pending_damage` to `Exceeded`, discarding row spans the
  sibling `CellChanged`s just recorded. *Rev 2:* grounded by the
  interaction trace — see "Interaction model" §4.
- Doc drift: `rendering-and-damage.md` §3 claims `markRowsDamaged` is
  never called; `subscribe.rs:101/:104` calls it.

### F8 — Hazard to verify (possible bug)

On a `Blitted` frame, a pane demoted to full `render_pane`
(`renderer/mod.rs:332/345`) takes the fingerprint path and could return
`Skip`/`Rows` against a painted tree describing **pre-shift** pixels.
*Rev 2:* the bridge-failure door of this hazard was already closed by
`unshiftable_pane_is_safe` (SESSION.md 2026-07-24) — but that guard has
no test, and the no-failure `Skip`/`Rows` door remains unproven either
way. Pin both in Stage 0.

## Rev 2 — reconciliation with the transactional review

The transactional review is the deeper diagnosis: dirty intent split
across two `PaintGate`s + signals + side-band state, no commit boundary,
and three blockers this doc missed — **held viewport attempts still
present + advance `last_frame` + clear pending work** (Blocker 1, already
sighted in SESSION.md 2026-07-23 "blit-abort does not hold `last_frame`"
— two independent sightings, zero tests), **resize mutates backing stores
without raising work** (Blocker 2), **playback never presents the grid
back buffer** (Blocker 3). All three accepted.

How Rev 1's proposal maps onto it:

| Rev 1 | Transactional review | Resolution |
|---|---|---|
| Move 1 — one fetch/commit funnel; fingerprint commit after strips | §5 side-effect-free prepare, §7 commit owns fingerprint state, Stage 6 "measure before touching fingerprints" | **Deferred to Stage 6.** The F1 cliff motivates it but is not the measured cost |
| Move 2 — merge Damage regime into SlotsReuse | §4 `FramePlan`: strategies become `GridWork::{None,Fresh,Panes,Rows,Blit}` inside one plan | **Converges.** The regime dissolves in Stage 3; `Rows` becomes plan work, `markRowsDamaged` API unchanged. Rev 1's test-impact inventory still applies |
| Move 3 — deletions & small fixes | VIEWPORT → Stage 2; arm/overlay drift → Stage 4; doc sync → Stage 7 | **Absorbed** |
| F3/F4 duplication inventory | §5 `PreparedBlitPane`, Stage 5 renderer shell | Feeds those stages |
| F8 hazard | not covered | **Added to Stage 0 pin list** |

### Measured evidence (SESSION.md FrameTrace, browser, 2026-07-25)

These measurements re-weight the priorities toward **fetch traffic, not
paint**:

- One cell edit: `SlotsReuse[CONTENT|OVERLAY] tl:skip tr:skip bl:skip
  br:rows1/1 fetched=2052` — the *paint* is already minimal; the cost is
  2052 cell slots crossing the bridge because `CellChanged` +
  `CalculationUpdated` in one batch poisons row damage into `SlotsReuse`.
- The 55 ms scroll spike is a `Viewport` frame degrading to `br:FULL
  fetched=4256` via `shift_is_safe`'s equal-row-count requirement
  (`IncompatibleRange` on a partially-visible edge row) — not F1's
  post-blit cliff.
- Invariant I1 (SESSION.md 2026-07-24): a fingerprint `Skip` saves the
  five-pass walk but never the four bulk fetches.

This is exactly what the transaction's trace/commit boundary (Stage 6)
is for: measure fetch separately from paint before changing fingerprint
or cache behavior.

## Interaction model — the grid contract

Owner's rule: *a user interaction on the base grid either edits content
or changes the view — never both; the exception is autofill, which must
route via the overlay path.*

Verified against the input layer 2026-07-28 (code traced, comments
distrusted):

- **Holds strictly** for paste, delete, undo/redo, fill-down, Esc,
  click/shift-click, point-mode ref picking, sheet switch — all
  single-class emissions.
- **Autofill already satisfies the exception as stated**: drag repaints
  via overlay diffing only (`subscribe.rs:47` `overlay_changed`; no engine
  content setter), edge auto-scroll is a view-only tick that deliberately
  does not arm `scroll_into_view` (`mousemove.rs:72-98`), and release
  emits a content-only batch (`mouseup.rs:26-54` — no nav event, selection
  unchanged). No work needed here.
- **The real dual-effect interaction is commit-then-move** (Enter/Tab,
  `edit.rs:190-227`): content mutation + `nav_arrow` + `scroll_into_view`
  arm, emitting the codebase's **only** content+nav batch (`edit.rs:224`).
  This is Excel's Enter semantics — inherent, not a wiring bug. Weaker
  dual cases: header resize drag (persisted layout + view geometry,
  `mousemove.rs:214/244`) and drag-select edge auto-scroll (overlay+view,
  no content).
- Excel-modal exclusivity is enforced **by construction only on the
  keyboard path** (`classify.rs:44` early-returns into the editing arm,
  making `NavAction` unreachable while editing). Mouse-path exclusivity is
  emergent. Incidental bug found and logged in SESSION.md: click-away
  during edit discards the buffer without committing (`click.rs:200`).

### Design consequences for the transactional pipeline

1. `PendingWork { view, content, overlay }` maps 1:1 onto the interaction
   classes; single-class ticks are the dominant case, so the planner's
   fast paths reflect reality instead of defending against hypothetical
   mixes.
2. Content+view co-arrival has **exactly one producer**. The planner can
   treat `content ∧ view` as a named, ordered case — content lands
   against the committed view, then the view moves — instead of every
   path re-deriving safety geometrically. Near term the combined case
   plans a conservative rebuild (current behavior); sequencing
   band-repaint → blit inside one transaction is a Stage 6 candidate,
   only if measurements justify it.
3. `view_changed` (transactional "wake-up vocabulary" item) gets a
   concrete producer inventory: the five `scroll_into_view` arm sites,
   wheel/nav paths, `autoscroll_tick`, and the freeze-clamp writeback
   (`raf_loop.rs:147-157`). `scroll_into_view` is a `StoredValue<bool>`
   side-channel today; fold it into the typed view intent rather than a
   flag the rAF body polls.
4. The Damage-poisoning tax on every edit (measured above) is the
   batch-level shadow of commit-then-move: `CalculationUpdated` is
   un-rowed, so the rowed `CellChanged` beside it degrades. Under
   `ContentWork` merge rules that degradation is correct; the real fix is
   a rowed recalc diff from the IronCalc bridge (open question — verify
   the event can carry affected addresses before assuming it).

## Revised plan

Adopt the transactional review's recommendation: **Stages 0–1
immediately**; review the `PendingWork`/`FramePlan` type shapes before
Stage 2. Additions from this doc:

- **Stage 0 pin-list additions:**
  8. F8 — a demoted pane on a `Blitted` frame must not fingerprint-`Skip`
     (or partial-`Rows`) over shifted pixels; fixture: pane in
     `stale_panes` with a cold cache (template per SESSION.md 2026-07-24).
  9. Commit-then-move regression — type + Enter on the bottom visible row
     paints correctly (the canonical content+view collision, and the only
     content+nav batch producer).
- **Stage 2:** implement `view_changed` from the producer inventory in
  "Design consequences" §3.
- **Stage 5 input:** the F3/F4 duplication inventory.
- **Stage 6 measurement list:** I1 fetch-on-Skip; `IncompatibleRange`
  ±1-edge-row strip acceptance (gated on the trace reporting
  `unshift(...,range)` not `,cold`, per SESSION.md); F1 post-scroll Full
  cliff; Damage-vs-SlotsReuse fetch ratio (Rev 1 Move 2's premise).

### Superseded

Rev 1's Moves 1–3, its "Suggested order", and its "Explicitly kept —
`decide()` cascade shape" are superseded by the transactional stages as
mapped above (the classifier/planner replaces `decide()`). Findings
F1–F8 remain the evidence base; the Rev 1 test-impact inventory remains
valid for the Stage 3 regime dissolution: 6 hard-break tests in
`orchestrator_regimes.rs` re-pointed; the 8 `row_band_repaint_*` +
3 `lifecycle_*` + 3 wasm raster tests key off the plan, not the tag, and
must pass unchanged.
