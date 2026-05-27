# Automation Panel — Design

**Date**: 2026-05-12
**Status**: Spec — awaiting user review
**Author**: brainstorm session, Opus 4.7

---

## Summary

A right-side slide-in panel that lets the user define a list of rules that
periodically mutate one cell each. Each rule targets either an absolute cell
address or a single-cell named range, and adds a signed `step` to the current
value on every interval tick. A single global Start/Stop drives all enabled
rules; the first failing rule stops the entire ticker and surfaces an error
banner.

Scope is deliberately narrow: increment/decrement only, one numeric cell per
rule, no bounds/wrap, no scheduling beyond fixed intervals.

## Goals

- Let the user create, edit, and delete inc/dec rules without touching the grid
- Persist the rule list per workbook so reload restores configuration
- Reuse existing infrastructure: `try_mutate`, `EventBus`, `WorkbookState`,
  `Split<T>`, `leptos-use`'s `use_interval_fn`, the `Modal`/drawer CSS patterns
- Provide a loud failure mode (stop everything, banner) so prototype bugs are
  obvious

## Non-goals

- Random / formula-driven step values
- Per-rule schedules (cron, "between 9 and 5", etc.)
- Multi-cell targets, ranges, or arrays
- Undo of automation writes (they pass through ironcalc like any other edit;
  ironcalc's existing undo applies, but we don't group ticks)
- Bounds, wrap-around, min/max constraints

---

## Architecture

```
WorkbookState (existing) ─────────────────────────────────────────────────┐
  ├─ automation_panel_open  : Split<bool>                                  │
  ├─ automation_rules       : Split<Vec<AutomationRule>>                   │
  ├─ automation_running     : Split<bool>                                  │
  └─ automation_error       : Split<Option<String>>                        │
                                                                          │
storage.rs (existing) ────────────────────────────────────────────────────┤
  └─ serializes automation_rules with the workbook payload                 │
     (panel_open / running / error are NEVER persisted)                    │
                                                                          │
src/automation/ (new) ────────────────────────────────────────────────────┤
  ├─ types.rs    : AutomationTarget, AutomationRule, TickError             │
  └─ ticker.rs   : install_ticker(), drive_one_tick(), fire_rule()         │
                  uses use_interval_fn @ 100ms granularity                 │
                                                                          │
src/components/automation_panel/ (new) ───────────────────────────────────┤
  ├─ mod.rs           : <AutomationPanel /> — shell, slide animation       │
  ├─ rule_row.rs      : <RuleRow rule_id=… />                              │
  ├─ rule_editor.rs   : inline editor inside the row                       │
  └─ target_picker.rs : Cell ↔ NamedRange radio + input                    │
                                                                          │
src/app.rs (modified) ────────────────────────────────────────────────────┤
  └─ mounts <AutomationPanel /> alongside <LeftDrawer /> and <Workbook />  │
                                                                          │
src/components/toolbar/ (modified) ───────────────────────────────────────┘
  └─ adds a button that toggles state.automation_panel_open
```

The ticker writes via `try_mutate(EvaluationMode::Immediate, …)`, which already
emits `ContentEvent` on success, so canvas repaint and debounced auto-save fire
through the existing pipeline with no new code.

---

## Data model

```rust
// src/automation/types.rs

pub enum AutomationTarget {
    Cell { sheet: u32, row: i32, col: i32 },  // exact field names follow whichever
                                              // address type is already used by
                                              // src/input/formula_input.rs
    NamedRange(String),                       // resolved each tick via
                                              // model.get_defined_name_list()
}

pub struct AutomationRule {
    pub id: u64,               // monotonic, assigned on create; stable <For> key
    pub label: String,         // free text; empty allowed; used in error banner
    pub target: AutomationTarget,
    pub step: f64,             // signed: +N = inc, -N = dec (no Op enum)
    pub interval_ms: u32,      // minimum 100, enforced in the editor
    pub enabled: bool,         // per-rule toggle; ticker only fires enabled rules
    #[serde(skip)]
    pub last_tick_ms: f64,     // performance.now() of last successful fire
}
```

Design choices:
- `AutomationTarget` is an `enum`, not flat fields with `Option`: illegal states
  (both filled, neither filled) are unrepresentable.
- Signed `step` instead of `Op::{Inc,Dec}` + magnitude: one field, one input,
  fewer matches.
- `last_tick_ms` is `#[serde(skip)]` — meaningless after a reload.
- `id` is the key for `<For>`, so reorder/edit doesn't blow away DOM nodes.

---

## State placement and persistence

All four signals live on `WorkbookState` (in `src/state.rs`):

```rust
pub(crate) automation_panel_open: Split<bool>,
pub(crate) automation_rules: Split<Vec<AutomationRule>>,
pub(crate) automation_running: Split<bool>,
pub(crate) automation_error: Split<Option<String>>,
```

Initialized in `WorkbookState::new(...)` to
`false / Vec::new() / false / None`.

`storage.rs` extends the workbook payload to include `automation_rules`. The
other three signals are session state and are never serialized — on reload, the
panel opens closed, the ticker is stopped, and the error banner is empty.

Rule edits do not flow through `EventBus`. Instead, the editor calls the
existing debounced save handle directly when the rule list mutates. The handle
lives in `app.rs` today; the implementation plan is to expose it via context
(smaller blast radius than adding a fourth `EventBus` category).

No new `SpreadsheetAction` variants. Start/Stop and rule edits stay outside the
keyboard pipeline because they are not grid actions. A keyboard shortcut can
be added later by extending `classify_key()` per `docs/adding-actions.md`.

---

## UI

### Mount

In `src/app.rs`:

```rust
view! {
    <div id="app">
        <LeftDrawer />
        <Workbook />
        <AutomationPanel />
    </div>
}
```

`<AutomationPanel>` is always in the DOM (no `<Show>`), so the CSS transform
animates cleanly. The shell is `position: fixed; right: 0; top: 0; bottom: 0;
width: 320px;` with `transform: translateX(100%)` when closed and
`translateX(0)` when open, transitioning over ~200 ms.

### Trigger

Add one toolbar button in `src/components/toolbar/mod.rs`:

```rust
<button class="tb-btn" on:click=toggle_panel title="Automation">"⚙ Auto"</button>
```

`toggle_panel` flips `state.automation_panel_open`.

### Panel layout

```
┌─ Automation ─────────────────────[ × ]┐
│  [ ▶ Start ]   [ + Rule ]              │
│                                        │
│  ⚠ "counter": cell is not numeric  [×] │  (only when automation_error is Some)
│                                        │
│  ┌──────────────────────────────────┐  │
│  │ ☑ [counter_____]            [🗑] │  │
│  │ Target: ◉ Cell  ○ Named range    │  │
│  │         [Sheet1!B5________]      │  │
│  │ Step: [ +1.0 ]   Every: [1000]ms │  │
│  └──────────────────────────────────┘  │
│  ┌──────────────────────────────────┐  │
│  │ ☐ [draindown__]             [🗑] │  │
│  │ Target: ○ Cell  ◉ Named range    │  │
│  │         [my_named_range___]      │  │
│  │ Step: [ -0.5 ]   Every: [ 500]ms │  │
│  └──────────────────────────────────┘  │
│                                        │
│  (empty state when list is empty)      │
└────────────────────────────────────────┘
```

Behavior:
- `▶ Start` swaps to `⏸ Stop` while `automation_running` is true; disabled when
  the rule list is empty
- `+ Rule` appends a default-valued rule and assigns a fresh `id`
- The error banner appears only when `automation_error.get().is_some()`; the
  `[×]` clears the error
- The rule list is a `<For each=rules key=|r| r.id let:rule>` of `<RuleRow>`s
- Each row edits its fields with inline inputs; changes write back to the
  signal immediately
- The cell input parses A1 via the existing parser used by
  `src/input/formula_input.rs`. Invalid input shows a red outline and prevents
  the rule from being enabled
- `interval_ms < 100` is rejected by the input
- 🗑 deletes the rule. Mid-tick deletion is safe (see ticker design)

CSS lives at `style/automation_panel.css`, imported alongside the other
component styles per `docs/building-components.md`.

---

## Ticker

A single driver, one `use_interval_fn` at fixed granularity, walks the rule
list and fires every enabled+due rule. Started by an `Effect` watching
`automation_running`.

```rust
// src/automation/ticker.rs

const DRIVER_GRANULARITY_MS: u64 = 100;

pub fn install_ticker() {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let UseIntervalFnReturn { pause, resume, .. } =
        use_interval_fn(
            move || drive_one_tick(state, model),
            DRIVER_GRANULARITY_MS,
        );

    Effect::new(move |_| {
        if state.automation_running.get() { resume(); } else { pause(); }
    });
}

fn drive_one_tick(state: WorkbookState, model: ModelStore) {
    let now = performance_now_ms();
    let rules_snapshot = state.automation_rules.get_untracked();

    for rule in rules_snapshot {
        if !rule.enabled { continue; }
        if now - rule.last_tick_ms < rule.interval_ms as f64 { continue; }

        match fire_rule(&rule, &model) {
            Ok(()) => {
                state.automation_rules.update(|rs| {
                    if let Some(r) = rs.iter_mut().find(|r| r.id == rule.id) {
                        r.last_tick_ms = now;
                    }
                });
            }
            Err(e) => {
                state.automation_error.set(Some(format!("{}: {}", rule.label, e)));
                state.automation_running.set(false);
                return;
            }
        }
    }
}

fn fire_rule(rule: &AutomationRule, model: &ModelStore) -> Result<(), TickError> {
    // resolve_target: for Cell -> direct; for NamedRange -> look up via
    // model.get_defined_name_list(), parse the formula, demand a single-cell
    // result. Returns a concrete (sheet, row, col) or a TickError.
    let addr = resolve_target(&rule.target, model)?;

    // read_numeric: reads the cell value; returns TickError::IsFormula if the
    // cell holds a formula, TickError::NotNumeric if non-numeric, else f64.
    let current: f64 = model.with_value(|m| read_numeric(m, addr))?;
    let next = current + rule.step;

    try_mutate(model, EvaluationMode::Immediate, |m| {
        m.set_user_input(addr.sheet, addr.row, addr.col, &next.to_string())
    })?;

    Ok(())
}
```

`install_ticker()` is called once from `<AutomationPanel>`'s `setup`, so the
timer's lifetime is tied to the panel's owner (which is the app root — the
panel mounts always).

### Error types

```rust
pub enum TickError {
    TargetNotFound,        // named range deleted or A1 out of bounds
    TargetNotSingleCell,   // named range expanded to a range
    NotNumeric,            // cell holds text, is empty, or otherwise non-numeric
    IsFormula,             // cell starts with '=' — refuse to overwrite
    MutationFailed(String),// ironcalc returned Err from set_user_input
}
```

Each variant maps to a human-readable string surfaced in the error banner.

### Why this shape

- `get_untracked()` for the snapshot: the driver is NOT a reactive consumer
  of the rule list. The Effect that pauses/resumes is the only reactive bridge.
- Snapshot-before-mutate: `Vec<AutomationRule>` is cloned by value into
  `rules_snapshot`, so deleting a rule mid-tick cannot invalidate the iterator.
- One write per fired rule (the `last_tick_ms` update): cheap; lets the UI
  show a "last fired" indicator later without changing the engine.
- First failure aborts the whole tick (`return`): consistent with the
  "stop the entire ticker" decision, and avoids cascading errors.

---

## Testing

Three layers, scaled to risk:

**Pure-logic tests** (`src/automation/ticker_test.rs`, `cargo test`):
- `fire_rule` cell-target happy path: seed a number, fire, assert new value
- `fire_rule` named-range happy path: create a single-cell named range, fire
  via `AutomationTarget::NamedRange(...)`, assert update
- Per-variant `TickError` coverage: multi-cell named range, deleted name,
  text cell, formula cell, deleberately-broken `set_user_input` call
- `drive_one_tick`: 3 rules with mixed `enabled`/due, fast-forwarded clock,
  asserts only enabled+due fire, `last_tick_ms` advances, others untouched
- `drive_one_tick` stop-on-failure: rule #2 fails, rule #1 fires, rule #3 is
  not attempted, `running=false`, `error` set

**Property test** (quickcheck, optional):
- For any `(start, step, count)` not overflowing: simulating `count` ticks
  produces `start + step*count`

**Component tests** (`src/test/automation_panel_test.rs`,
`wasm_bindgen_test`):
- Mount, click `+ Rule`, assert row renders
- Edit step, click Start, assert `automation_running` flips
- Toggle `enabled` mid-run, assert next tick skips it
- Inject non-numeric target, click Start, assert banner shows the label and
  `running` is false

**Manual smoke** (recorded in commit message):
- "counter B1 +1 every 500ms" — watch B1 climb on the canvas
- Reload: panel closed, ticker stopped, rules restored
- Switch workbook: rules switch with it

**Out of scope**:
- Timer-drift cross-browser benchmarks (we accept ~10 ms wobble)
- Driver micro-benchmarks (idle cost is one closure call per 100 ms)

---

## Implementation order

A rough sequencing for the writing-plans pass; not commitments:

1. `src/automation/types.rs` + unit tests for `AutomationTarget`/`AutomationRule`
   serde round-trips
2. `WorkbookState` fields + `Split<T>` initializers
3. `storage.rs` extension for `automation_rules` persistence + a migration
   default for workbooks saved before the field existed
4. `src/automation/ticker.rs` with `fire_rule` + unit tests against a seeded
   model
5. `drive_one_tick` + clock injection + unit tests
6. `<AutomationPanel>` shell + slide CSS + toolbar trigger
7. `<RuleRow>` + inline editing + target picker + A1 validation
8. Error banner + stop-on-failure wiring
9. Component tests via `wasm_bindgen_test`
10. Manual smoke pass; commit

---

## Open questions for implementation

- Should `last_tick_ms` be visible in the row UI as a "last fired" indicator?
  Default: no — save for a follow-up.
- Exact field names of the address type. The spec uses
  `Cell { sheet, row, col }` as a placeholder shape; the implementation will
  use whichever existing type in `coord.rs` is already plumbed through
  `set_user_input` (avoids one more wrapper).
