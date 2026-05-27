# Automation Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a right-side slide-in panel that runs inc/dec automation rules against single cells or single-cell named ranges, driven by one shared ticker, with persistent per-workbook rule storage.

**Architecture:** A new `src/automation/` module owns the pure engine (types, target resolution, value read, tick driver). State lives on `WorkbookState` as `Split<T>` signals. Rules persist to a per-workbook sidecar localStorage key (independent of the ironcalc binary workbook payload). UI is a slide-in `<AutomationPanel>` mounted from `src/app.rs`, triggered by a button in `src/components/toolbar/mod.rs`. The driver is one `use_interval_fn` at 100 ms granularity, paused/resumed by an `Effect` watching `automation_running`.

**Tech Stack:** Rust + Leptos 0.8 + `leptos-use` 0.18 (`use_interval_fn`) + `ironcalc_base::UserModel` + `gloo_storage::LocalStorage` + `serde_json`.

---

## Spec divergence

The spec said rules would be serialized with the workbook payload. Investigation showed the workbook is stored as the ironcalc binary format under a single `models` key — there is no JSON sidecar to extend cleanly. The plan therefore uses a **separate localStorage key per workbook UUID** (`rustycalc_automation_<uuid>`) holding a JSON-encoded `Vec<AutomationRule>`. Save on every mutation; load on workbook load.

---

## File structure

| Path | Purpose | Created/Modified |
|------|---------|------------------|
| `src/automation/mod.rs` | Module entry; re-exports types and `install_ticker` | Create |
| `src/automation/types.rs` | `AutomationTarget`, `AutomationRule`, `TickError` | Create |
| `src/automation/storage.rs` | Sidecar localStorage load/save keyed by `WorkbookId` | Create |
| `src/automation/resolve.rs` | `resolve_target`: `AutomationTarget` → `(sheet, row, col)` | Create |
| `src/automation/read.rs` | `read_numeric`: pure read returning `f64` or `TickError` | Create |
| `src/automation/ticker.rs` | `fire_rule`, `drive_one_tick`, `install_ticker` | Create |
| `src/components/automation_panel/mod.rs` | `<AutomationPanel>` shell, slide-in, header, controls | Create |
| `src/components/automation_panel/rule_row.rs` | `<RuleRow>` inline editor row | Create |
| `src/components/automation_panel/target_picker.rs` | Cell ↔ NamedRange radio + input | Create |
| `style/automation_panel.css` | Panel styles | Create |
| `src/components/mod.rs` | Register `automation_panel` | Modify |
| `src/state.rs` | Add four `automation_*` signals to `WorkbookState` | Modify |
| `src/lib.rs` (or `src/main.rs`) | Register `automation` module | Modify |
| `src/app.rs` | Mount `<AutomationPanel>`; load rules on workbook bootstrap | Modify |
| `src/components/toolbar/mod.rs` | Add "Automation" toggle button | Modify |

Module split rationale: pure-logic units (`types`, `resolve`, `read`, `ticker`) are testable without Leptos; UI units are in `components/`. Storage isolated so swap to a different persistence backend later is a one-file change.

---

## Notes for the implementing engineer

- **Never edit `Cargo.toml`.** All required crates (`leptos`, `leptos-use`, `serde`, `serde_json`, `gloo_storage`, `ironcalc_base`) are already declared. If something is missing, stop and ask the user.
- **No commits.** The user manages commits. Each task ends with verification, not `git commit`.
- **No bare `unwrap()` / `expect("")`.** Use `let-else`, `match`, or `?` with concrete error variants.
- **`set_user_input` signature:** `m.set_user_input(sheet: u32, row: i32, column: i32, value: &str) -> Result<(), …>` — verified against `src/model/frontend_model.rs:640`.
- **All mutations go through `try_mutate(model, EvaluationMode::Immediate, |m| …)`** — this gives the canvas, formula bar, and auto-save free.
- **Run `cargo check` then `cargo test --lib` after every meaningful change.** Stop and reassess after 3 consecutive failures.
- **Wasm-rendered DOM tests are out of scope** for this plan. Pure-logic tests live inline (`#[cfg(test)] mod tests`); UI is verified manually via `trunk serve` at the end.

---

### Task 1: Define `AutomationTarget`, `AutomationRule`, `TickError`

**Files:**
- Create: `src/automation/types.rs`
- Create: `src/automation/mod.rs`
- Modify: `src/lib.rs` (or `src/main.rs`, whichever declares modules)

- [ ] **Step 1: Create `src/automation/mod.rs`**

```rust
pub mod types;
```

- [ ] **Step 2: Create `src/automation/types.rs` with the three types**

```rust
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AutomationTarget {
    Cell { sheet: u32, row: i32, column: i32 },
    NamedRange(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: u64,
    pub label: String,
    pub target: AutomationTarget,
    pub step: f64,
    pub interval_ms: u32,
    pub enabled: bool,
    #[serde(skip)]
    pub last_tick_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TickError {
    TargetNotFound(String),
    TargetNotSingleCell(String),
    NotNumeric,
    IsFormula,
    MutationFailed(String),
}

impl std::fmt::Display for TickError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetNotFound(s) => write!(f, "target not found: {s}"),
            Self::TargetNotSingleCell(s) => write!(f, "named range \"{s}\" does not resolve to a single cell"),
            Self::NotNumeric => write!(f, "cell is not numeric"),
            Self::IsFormula => write!(f, "cell holds a formula; refusing to overwrite"),
            Self::MutationFailed(s) => write!(f, "ironcalc write failed: {s}"),
        }
    }
}

impl AutomationRule {
    pub fn new_default(id: u64) -> Self {
        Self {
            id,
            label: String::new(),
            target: AutomationTarget::Cell { sheet: 0, row: 1, column: 1 },
            step: 1.0,
            interval_ms: 1000,
            enabled: true,
            last_tick_ms: 0.0,
        }
    }
}
```

- [ ] **Step 3: Register `automation` module in the crate root**

Open `src/lib.rs` (or `src/main.rs`); find the existing list of `pub mod …;` declarations (e.g. near `pub mod components;`). Add:

```rust
pub mod automation;
```

- [ ] **Step 4: Add round-trip serde test inside `src/automation/types.rs`**

Append to the same file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_round_trips_through_json() {
        let rule = AutomationRule {
            id: 7,
            label: "counter".into(),
            target: AutomationTarget::Cell { sheet: 0, row: 5, column: 3 },
            step: -1.5,
            interval_ms: 500,
            enabled: true,
            last_tick_ms: 123_456.0,
        };

        let json = serde_json::to_string(&rule).unwrap();
        let back: AutomationRule = serde_json::from_str(&json).unwrap();

        // last_tick_ms is #[serde(skip)] — must reset to 0.0 after round-trip.
        let expected = AutomationRule { last_tick_ms: 0.0, ..rule };
        assert_eq!(back, expected);
    }

    #[test]
    fn named_range_target_round_trips() {
        let rule = AutomationRule {
            id: 1,
            label: "tick".into(),
            target: AutomationTarget::NamedRange("count".into()),
            step: 1.0,
            interval_ms: 1000,
            enabled: false,
            last_tick_ms: 0.0,
        };
        let json = serde_json::to_string(&rule).unwrap();
        let back: AutomationRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rule);
    }
}
```

- [ ] **Step 5: Verify compilation and tests**

Run: `cargo check --lib`
Expected: clean, no warnings about `automation` module being unused (it's `pub`).

Run: `cargo test --lib automation::types::tests`
Expected: 2 tests passed.

---

### Task 2: Add `automation_*` signals to `WorkbookState`

**Files:**
- Modify: `src/state.rs:231-289` (the `WorkbookState` struct and its `new()` impl)

- [ ] **Step 1: Add imports near the top of `src/state.rs`**

After the existing `use crate::events::…;` block, add:

```rust
use crate::automation::types::AutomationRule;
```

- [ ] **Step 2: Add four fields to the `WorkbookState` struct**

Locate the `pub struct WorkbookState` definition (around line 231). Immediately before the closing `}`, add:

```rust
    pub(crate) automation_panel_open: Split<bool>,
    pub(crate) automation_rules: Split<Vec<AutomationRule>>,
    pub(crate) automation_running: Split<bool>,
    pub(crate) automation_error: Split<Option<String>>,
```

- [ ] **Step 3: Initialize the new fields in `WorkbookState::new`**

Locate the `Self { … }` block inside `pub fn new(events: EventBus) -> Self`. Before the closing `}`, add:

```rust
            automation_panel_open: Split::new(false),
            automation_rules: Split::new(Vec::new()),
            automation_running: Split::new(false),
            automation_error: Split::new(None),
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --lib`
Expected: clean.

---

### Task 3: Sidecar localStorage for rules

**Files:**
- Create: `src/automation/storage.rs`
- Modify: `src/automation/mod.rs` (export the new module)

- [ ] **Step 1: Add `pub mod storage;` to `src/automation/mod.rs`**

The file should now read:

```rust
pub mod storage;
pub mod types;
```

- [ ] **Step 2: Create `src/automation/storage.rs`**

```rust
use gloo_storage::{LocalStorage, Storage};

use crate::automation::types::AutomationRule;
use crate::storage::WorkbookId;

fn key_for(uuid: &WorkbookId) -> String {
    format!("rustycalc_automation_{uuid}")
}

pub fn load(uuid: &WorkbookId) -> Vec<AutomationRule> {
    LocalStorage::get::<Vec<AutomationRule>>(&key_for(uuid)).unwrap_or_default()
}

pub fn save(uuid: &WorkbookId, rules: &[AutomationRule]) {
    if let Err(e) = LocalStorage::set(&key_for(uuid), rules) {
        web_sys::console::warn_1(
            &format!("[rustycalc automation] save failed: {e}").into(),
        );
    }
}

pub fn clear(uuid: &WorkbookId) {
    LocalStorage::delete(&key_for(uuid));
}
```

- [ ] **Step 3: Add a pure unit test for the key format**

Append to `src/automation/storage.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // We can't easily test LocalStorage without a wasm runtime, but we can
    // assert the key format is stable so consumers won't collide.
    #[test]
    fn key_for_is_namespaced() {
        let uuid: WorkbookId = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        assert_eq!(
            key_for(&uuid),
            "rustycalc_automation_550e8400-e29b-41d4-a716-446655440000"
        );
    }
}
```

- [ ] **Step 4: Verify compilation and tests**

Run: `cargo check --lib`
Expected: clean.

Run: `cargo test --lib automation::storage::tests`
Expected: 1 test passed.

---

### Task 4: `resolve_target` for `Cell` variant

**Files:**
- Create: `src/automation/resolve.rs`
- Modify: `src/automation/mod.rs`

- [ ] **Step 1: Register module**

Update `src/automation/mod.rs`:

```rust
pub mod resolve;
pub mod storage;
pub mod types;
```

- [ ] **Step 2: Create `src/automation/resolve.rs` with the `Cell` arm only**

```rust
use ironcalc_base::UserModel;

use crate::automation::types::{AutomationTarget, TickError};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedAddress {
    pub sheet: u32,
    pub row: i32,
    pub column: i32,
}

pub fn resolve_target(
    target: &AutomationTarget,
    model: &UserModel<'static>,
) -> Result<ResolvedAddress, TickError> {
    match target {
        AutomationTarget::Cell { sheet, row, column } => Ok(ResolvedAddress {
            sheet: *sheet,
            row: *row,
            column: *column,
        }),
        AutomationTarget::NamedRange(name) => {
            // Implemented in Task 5.
            let _ = (name, model);
            Err(TickError::TargetNotFound("not yet implemented".into()))
        }
    }
}
```

- [ ] **Step 3: Add a unit test for the `Cell` arm**

Append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::UserModel;

    fn empty_model() -> UserModel<'static> {
        UserModel::new_empty("test", "en", "UTC").unwrap()
    }

    #[test]
    fn resolves_explicit_cell_target() {
        let m = empty_model();
        let target = AutomationTarget::Cell { sheet: 0, row: 5, column: 3 };
        let r = resolve_target(&target, &m).unwrap();
        assert_eq!(r, ResolvedAddress { sheet: 0, row: 5, column: 3 });
    }
}
```

- [ ] **Step 4: Verify**

Run: `cargo check --lib && cargo test --lib automation::resolve::tests`
Expected: 1 test passed.

---

### Task 5: `resolve_target` for `NamedRange` variant

**Files:**
- Modify: `src/automation/resolve.rs`

- [ ] **Step 1: Look up the named-range API**

Search for the helper used by the existing modal:

Run: `rg -n 'get_defined_name_list|defined_name_list' src/ --type rust`

You should see calls returning `Vec<DefinedNameS>` where `DefinedNameS = (String, Option<u32>, String)` — `(name, scope, formula)`. The formula is something like `Sheet1!$B$5` for a single cell, or `Sheet1!$B$5:$C$7` for a range.

If the exact accessor differs, follow the call sites in `src/components/named_ranges/form.rs` for the canonical pattern.

- [ ] **Step 2: Implement single-cell named-range resolution**

Replace the `NamedRange` arm in `src/automation/resolve.rs` with logic that:
1. Walks `UserModel::get_defined_name_list()` for a name match (case-insensitive).
2. Parses the formula via the same parser used by `src/input/formula_input.rs` / `src/input/formula_analysis.rs`. The cleanest entry point is the parser already present there — adapt the helper used by `formula_analysis.rs`.
3. Returns `ResolvedAddress` only if the parsed reference is a single cell (range with `r1 == r2 && c1 == c2`). Otherwise returns `TickError::TargetNotSingleCell(name.clone())`.

The exact parser call depends on the project's helpers — verify against `src/input/formula_analysis.rs:240-260` for the pattern used there. Do not invent new parser entry points.

```rust
// Sketch — adapt parser call to whatever formula_analysis.rs uses.
AutomationTarget::NamedRange(name) => {
    let needle = name.to_ascii_lowercase();
    let entry = model
        .get_defined_name_list()
        .into_iter()
        .find(|(n, _, _)| n.to_ascii_lowercase() == needle)
        .ok_or_else(|| TickError::TargetNotFound(name.clone()))?;

    let (_, _scope, formula) = entry;
    // TODO during implementation: use the project's existing parser helper
    // (mirrors src/input/formula_analysis.rs) to turn `formula` into a
    // resolved sheet+r1+c1+r2+c2. Reject anything where r1 != r2 || c1 != c2
    // with TickError::TargetNotSingleCell(name.clone()).
    parse_single_cell_ref(&formula, model)
        .ok_or_else(|| TickError::TargetNotSingleCell(name.clone()))
}
```

`parse_single_cell_ref` should be a small private helper in this same file that returns `Option<ResolvedAddress>`. Keep it private; the test below pins behaviour.

- [ ] **Step 3: Tests for both branches**

Append to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn named_range_missing_returns_target_not_found() {
        let m = empty_model();
        let target = AutomationTarget::NamedRange("ghost".into());
        let err = resolve_target(&target, &m).unwrap_err();
        assert!(matches!(err, TickError::TargetNotFound(ref n) if n == "ghost"));
    }

    #[test]
    fn named_range_single_cell_resolves() {
        let mut m = empty_model();
        // Use the same API as src/components/named_ranges/form.rs:125
        m.create_defined_name("counter", None, "Sheet1!$B$5").unwrap();

        let target = AutomationTarget::NamedRange("counter".into());
        let r = resolve_target(&target, &m).unwrap();
        // Sheet1 = 0, $B$5 = (4, 1) in 0-indexed row/col — adjust if the
        // project uses 1-indexed addresses; cross-check with
        // ContentEvent::CellChanged in src/events.rs.
        assert_eq!(r.sheet, 0);
        // Pin the exact row/column once you verify indexing convention.
    }

    #[test]
    fn named_range_multi_cell_returns_target_not_single_cell() {
        let mut m = empty_model();
        m.create_defined_name("blob", None, "Sheet1!$B$5:$C$7").unwrap();
        let target = AutomationTarget::NamedRange("blob".into());
        let err = resolve_target(&target, &m).unwrap_err();
        assert!(matches!(err, TickError::TargetNotSingleCell(_)));
    }
```

- [ ] **Step 4: Verify**

Run: `cargo check --lib && cargo test --lib automation::resolve::tests`
Expected: 4 tests passed (1 from Task 4 + 3 new). If the single-cell test fails because of an indexing-convention mismatch, fix the test expectations using the convention from `src/coord.rs` and `src/events.rs::ContentEvent::CellChanged` — do not change the implementation to "match" the test.

---

### Task 6: `read_numeric`

**Files:**
- Create: `src/automation/read.rs`
- Modify: `src/automation/mod.rs`

- [ ] **Step 1: Register module**

```rust
pub mod read;
pub mod resolve;
pub mod storage;
pub mod types;
```

- [ ] **Step 2: Create `src/automation/read.rs`**

```rust
use ironcalc_base::UserModel;

use crate::automation::resolve::ResolvedAddress;
use crate::automation::types::TickError;

pub fn read_numeric(
    model: &UserModel<'static>,
    addr: ResolvedAddress,
) -> Result<f64, TickError> {
    // Step 1: get the raw input string (NOT the formatted value).
    //
    // The accessor lives on UserModel. Use whichever of these matches the
    // existing code in src/model/frontend_model.rs:
    //   - get_cell_content_at(sheet, row, column)
    //   - get_input_at(sheet, row, column)
    //   - read_cell(sheet, row, column)
    //
    // Verify with `rg -n 'fn get_(cell|input)' src/model/` before writing.
    let raw: String = model
        .get_cell_content(addr.sheet, addr.row, addr.column)
        .map_err(|e| TickError::MutationFailed(format!("read: {e}")))?;

    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err(TickError::NotNumeric);
    }
    if trimmed.starts_with('=') {
        return Err(TickError::IsFormula);
    }
    trimmed.parse::<f64>().map_err(|_| TickError::NotNumeric)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironcalc_base::UserModel;

    fn model_with(value: &str) -> UserModel<'static> {
        let mut m = UserModel::new_empty("t", "en", "UTC").unwrap();
        m.set_user_input(0, 1, 1, value).unwrap();
        m
    }

    fn b1() -> ResolvedAddress {
        ResolvedAddress { sheet: 0, row: 1, column: 1 }
    }

    #[test]
    fn reads_numeric_input() {
        let m = model_with("42.5");
        assert_eq!(read_numeric(&m, b1()).unwrap(), 42.5);
    }

    #[test]
    fn empty_cell_is_not_numeric() {
        let m = UserModel::new_empty("t", "en", "UTC").unwrap();
        assert_eq!(read_numeric(&m, b1()).unwrap_err(), TickError::NotNumeric);
    }

    #[test]
    fn text_is_not_numeric() {
        let m = model_with("hello");
        assert_eq!(read_numeric(&m, b1()).unwrap_err(), TickError::NotNumeric);
    }

    #[test]
    fn formula_is_rejected() {
        let m = model_with("=1+2");
        assert_eq!(read_numeric(&m, b1()).unwrap_err(), TickError::IsFormula);
    }
}
```

- [ ] **Step 3: Verify and adjust to actual API**

Run: `cargo check --lib`

If `get_cell_content` doesn't exist or has a different signature, search for the actual accessor:

Run: `rg -n 'fn get_(cell|input|text|formatted)' src/model/frontend_model.rs`

Adjust the call to match. Do not invent. If the accessor needs a different argument order (e.g. `(row, column, sheet)`), follow it.

Run: `cargo test --lib automation::read::tests`
Expected: 4 tests passed.

---

### Task 7: `fire_rule`

**Files:**
- Create: `src/automation/ticker.rs`
- Modify: `src/automation/mod.rs`

- [ ] **Step 1: Register module and create file skeleton**

`src/automation/mod.rs`:

```rust
pub mod read;
pub mod resolve;
pub mod storage;
pub mod ticker;
pub mod types;

pub use ticker::install_ticker;
```

- [ ] **Step 2: Create `src/automation/ticker.rs` with `fire_rule` only**

```rust
use ironcalc_base::UserModel;

use crate::automation::read::read_numeric;
use crate::automation::resolve::resolve_target;
use crate::automation::types::{AutomationRule, TickError};

pub(crate) fn fire_rule_on(
    rule: &AutomationRule,
    model: &mut UserModel<'static>,
) -> Result<(), TickError> {
    let addr = resolve_target(&rule.target, model)?;
    let current = read_numeric(model, addr)?;
    let next = current + rule.step;
    model
        .set_user_input(addr.sheet, addr.row, addr.column, &next.to_string())
        .map_err(|e| TickError::MutationFailed(e.to_string()))?;
    Ok(())
}
```

`fire_rule_on` operates on a borrowed `&mut UserModel` so it is testable without a `ModelStore`. The reactive wrapper (next task) will call it inside `try_mutate`'s closure.

- [ ] **Step 3: Add unit tests for the happy path and each error variant**

Append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::types::AutomationTarget;

    fn rule_at(sheet: u32, row: i32, column: i32, step: f64) -> AutomationRule {
        AutomationRule {
            id: 1,
            label: "t".into(),
            target: AutomationTarget::Cell { sheet, row, column },
            step,
            interval_ms: 100,
            enabled: true,
            last_tick_ms: 0.0,
        }
    }

    fn fresh() -> UserModel<'static> {
        UserModel::new_empty("t", "en", "UTC").unwrap()
    }

    #[test]
    fn fires_and_increments_existing_number() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "10").unwrap();
        let rule = rule_at(0, 1, 1, 1.5);

        fire_rule_on(&rule, &mut m).unwrap();

        let v = m.get_cell_content(0, 1, 1).unwrap();
        assert_eq!(v.trim().parse::<f64>().unwrap(), 11.5);
    }

    #[test]
    fn fires_with_negative_step_decrements() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "10").unwrap();
        let rule = rule_at(0, 1, 1, -3.0);
        fire_rule_on(&rule, &mut m).unwrap();
        let v = m.get_cell_content(0, 1, 1).unwrap();
        assert_eq!(v.trim().parse::<f64>().unwrap(), 7.0);
    }

    #[test]
    fn empty_cell_yields_not_numeric() {
        let mut m = fresh();
        let rule = rule_at(0, 1, 1, 1.0);
        assert_eq!(fire_rule_on(&rule, &mut m).unwrap_err(), TickError::NotNumeric);
    }

    #[test]
    fn formula_cell_is_refused() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "=1+2").unwrap();
        let rule = rule_at(0, 1, 1, 1.0);
        assert_eq!(fire_rule_on(&rule, &mut m).unwrap_err(), TickError::IsFormula);
    }
}
```

- [ ] **Step 4: Verify**

Run: `cargo check --lib && cargo test --lib automation::ticker::tests`
Expected: 4 tests passed.

---

### Task 8: `drive_one_tick` (engine, no Leptos yet)

**Files:**
- Modify: `src/automation/ticker.rs`

The driver walks a snapshot of `Vec<AutomationRule>` and fires every enabled+due rule. We keep it pure by taking the rules slice and a clock by value; the reactive wrapper in Task 9 will adapt this for `Split<T>`.

- [ ] **Step 1: Add `TickOutcome` and `drive_one_tick`**

Append to `src/automation/ticker.rs` (before the `#[cfg(test)]` block):

```rust
pub struct TickOutcome {
    /// Rule ids that fired successfully on this tick.
    pub fired_ids: Vec<u64>,
    /// First failure observed; aborts the tick.
    pub error: Option<(u64, TickError)>,
}

pub fn drive_one_tick(
    rules: &[AutomationRule],
    now_ms: f64,
    model: &mut UserModel<'static>,
) -> TickOutcome {
    let mut fired_ids = Vec::new();
    let mut error = None;

    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if now_ms - rule.last_tick_ms < rule.interval_ms as f64 {
            continue;
        }
        match fire_rule_on(rule, model) {
            Ok(()) => fired_ids.push(rule.id),
            Err(e) => {
                error = Some((rule.id, e));
                break; // first failure aborts the whole tick
            }
        }
    }

    TickOutcome { fired_ids, error }
}
```

- [ ] **Step 2: Add tests**

Inside the existing `tests` mod:

```rust
    fn enabled_rule(id: u64, addr: (i32, i32), interval: u32, step: f64) -> AutomationRule {
        AutomationRule {
            id,
            label: format!("r{id}"),
            target: AutomationTarget::Cell {
                sheet: 0,
                row: addr.0,
                column: addr.1,
            },
            step,
            interval_ms: interval,
            enabled: true,
            last_tick_ms: 0.0,
        }
    }

    #[test]
    fn fires_only_due_rules() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "0").unwrap();
        m.set_user_input(0, 2, 2, "0").unwrap();

        // r1 is due (last_tick_ms=0, interval=100, now=200).
        // r2 is not due (last_tick_ms=150, interval=100, now=200 -> 50 < 100).
        let mut r1 = enabled_rule(1, (1, 1), 100, 1.0);
        r1.last_tick_ms = 0.0;
        let mut r2 = enabled_rule(2, (2, 2), 100, 1.0);
        r2.last_tick_ms = 150.0;

        let out = drive_one_tick(&[r1, r2], 200.0, &mut m);
        assert_eq!(out.fired_ids, vec![1]);
        assert!(out.error.is_none());
    }

    #[test]
    fn skips_disabled_rules() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "0").unwrap();
        let mut r = enabled_rule(1, (1, 1), 100, 1.0);
        r.enabled = false;
        let out = drive_one_tick(&[r], 999.0, &mut m);
        assert!(out.fired_ids.is_empty());
        assert!(out.error.is_none());
    }

    #[test]
    fn stops_on_first_failure() {
        let mut m = fresh();
        m.set_user_input(0, 1, 1, "0").unwrap(); // r1 ok
        // r2 targets an empty cell -> NotNumeric.
        let r1 = enabled_rule(1, (1, 1), 100, 1.0);
        let r2 = enabled_rule(2, (5, 5), 100, 1.0);
        let r3 = enabled_rule(3, (1, 1), 100, 1.0);

        let out = drive_one_tick(&[r1, r2, r3], 999.0, &mut m);
        assert_eq!(out.fired_ids, vec![1]);
        assert!(matches!(out.error, Some((2, TickError::NotNumeric))));
        // r3 must NOT have fired — assert by checking the value didn't double.
        let v = m.get_cell_content(0, 1, 1).unwrap().trim().parse::<f64>().unwrap();
        assert_eq!(v, 1.0);
    }
```

- [ ] **Step 3: Verify**

Run: `cargo check --lib && cargo test --lib automation::ticker::tests`
Expected: 7 tests passed (4 from Task 7 + 3 new).

---

### Task 9: `install_ticker` (Leptos wrapper)

**Files:**
- Modify: `src/automation/ticker.rs`

This is the only Leptos-aware code in the engine. It reads `WorkbookState` via context, ties `automation_running` to `use_interval_fn`'s pause/resume, and on each fire writes back the updated `last_tick_ms`.

- [ ] **Step 1: Add imports at the top of `ticker.rs`**

```rust
use leptos::prelude::*;
use leptos_use::{use_interval_fn, UseIntervalFnReturn, utils::Pausable};

use crate::model::{try_mutate, EvaluationMode};
use crate::state::{ModelStore, WorkbookState};
```

If `Pausable` / `UseIntervalFnReturn` are in a different submodule of `leptos-use` 0.18, follow the rustc error — the path is exact in that crate version.

- [ ] **Step 2: Add `install_ticker`**

Append:

```rust
const DRIVER_GRANULARITY_MS: u64 = 100;

fn now_ms() -> f64 {
    leptos::prelude::window()
        .performance()
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// Install one shared interval driver. Lifetime is tied to the calling owner
/// (the `<AutomationPanel>` component). The interval is paused/resumed by an
/// `Effect` watching `WorkbookState::automation_running`.
pub fn install_ticker() {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let UseIntervalFnReturn { pause, resume, .. } = use_interval_fn(
        move || tick_once(state, model),
        DRIVER_GRANULARITY_MS,
    );

    // Start paused; the Effect below resumes when running flips true.
    pause();

    Effect::new(move |_| {
        if state.automation_running.get() {
            resume();
        } else {
            pause();
        }
    });
}

fn tick_once(state: WorkbookState, model: ModelStore) {
    let rules_snapshot = state.automation_rules.get_untracked();
    if rules_snapshot.is_empty() {
        return;
    }

    let now = now_ms();
    let mut outcome = TickOutcome { fired_ids: Vec::new(), error: None };

    // Single try_mutate covers the whole tick: pause evaluation once,
    // run every fire, then evaluate once.
    let _ = try_mutate::<()>(model, EvaluationMode::Immediate, |m| {
        outcome = drive_one_tick(&rules_snapshot, now, m);
        Ok(())
    });

    // Write back per-rule last_tick_ms for successful fires.
    if !outcome.fired_ids.is_empty() {
        state.automation_rules.update(|rs| {
            for r in rs.iter_mut() {
                if outcome.fired_ids.contains(&r.id) {
                    r.last_tick_ms = now;
                }
            }
        });
    }

    // First failure: surface the error and stop the global ticker.
    if let Some((failing_id, err)) = outcome.error {
        let label = rules_snapshot
            .iter()
            .find(|r| r.id == failing_id)
            .map(|r| r.label.clone())
            .unwrap_or_default();
        let msg = if label.is_empty() {
            format!("rule #{failing_id}: {err}")
        } else {
            format!("{label}: {err}")
        };
        state.automation_error.set(Some(msg));
        state.automation_running.set(false);
    }
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --lib`
Expected: clean. Fix any path issues for `UseIntervalFnReturn` / `Pausable` against the actual `leptos-use = "0.18"` API.

(No unit tests for `install_ticker` — it requires a Leptos runtime. Manual smoke covers it in Task 16.)

---

### Task 10: Register `automation_panel` component module

**Files:**
- Modify: `src/components/mod.rs`
- Create: `src/components/automation_panel/mod.rs` (empty shell)

- [ ] **Step 1: Add the entry to `src/components/mod.rs`**

After the existing `pub mod left_drawer;`, add:

```rust
pub mod automation_panel;
```

(Keep alphabetical ordering only if the existing file already enforces it.)

- [ ] **Step 2: Create an empty `src/components/automation_panel/mod.rs`**

```rust
use leptos::prelude::*;

#[component]
pub fn AutomationPanel() -> impl IntoView {
    view! { <div class="auto-panel" hidden=true /> }
}
```

- [ ] **Step 3: Verify**

Run: `cargo check --lib`
Expected: clean.

---

### Task 11: Panel shell — open/close, header, global controls, error banner, empty state

**Files:**
- Modify: `src/components/automation_panel/mod.rs`
- Create: `style/automation_panel.css`
- Modify: wherever component CSS files are imported (check `docs/building-components.md` step 4 and the existing pattern under `style/`)

- [ ] **Step 1: Replace the placeholder with the full shell**

```rust
use leptos::prelude::*;

use crate::automation::install_ticker;
use crate::automation::types::AutomationRule;
use crate::state::WorkbookState;

#[component]
pub fn AutomationPanel() -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Mount the ticker exactly once, with this panel as its owner.
    install_ticker();

    let open_class = move || {
        if state.automation_panel_open.get() {
            "auto-panel open"
        } else {
            "auto-panel"
        }
    };

    let on_close = move |_| state.automation_panel_open.set(false);

    let toggle_running = move |_| {
        state.automation_running.update(|v| *v = !*v);
        if state.automation_running.get_untracked() {
            state.automation_error.set(None);
        }
    };

    let add_rule = move |_| {
        state.automation_rules.update(|rs| {
            let id = next_rule_id(rs);
            rs.push(AutomationRule::new_default(id));
        });
    };

    let running_label = move || {
        if state.automation_running.get() { "⏸ Stop" } else { "▶ Start" }
    };

    let start_disabled = move || state.automation_rules.with(|rs| rs.is_empty());

    let dismiss_error = move |_| state.automation_error.set(None);

    view! {
        <aside class=open_class>
            <header class="auto-panel-header">
                <h2>"Automation"</h2>
                <button class="auto-panel-close" on:click=on_close title="Close">"×"</button>
            </header>

            <div class="auto-panel-controls">
                <button
                    class="auto-panel-start"
                    on:click=toggle_running
                    prop:disabled=start_disabled
                >
                    {running_label}
                </button>
                <button class="auto-panel-add" on:click=add_rule>"+ Rule"</button>
            </div>

            <Show when=move || state.automation_error.get().is_some()>
                <div class="auto-panel-error">
                    <span>{move || state.automation_error.get().unwrap_or_default()}</span>
                    <button on:click=dismiss_error title="Dismiss">"×"</button>
                </div>
            </Show>

            <div class="auto-panel-body">
                <Show
                    when=move || state.automation_rules.with(|rs| !rs.is_empty())
                    fallback=|| view! { <div class="auto-panel-empty">"No rules yet — click + Rule to create one."</div> }
                >
                    <ul class="auto-panel-list">
                        // RuleRow added in Task 12
                    </ul>
                </Show>
            </div>
        </aside>
    }
}

fn next_rule_id(existing: &[AutomationRule]) -> u64 {
    existing.iter().map(|r| r.id).max().unwrap_or(0) + 1
}
```

- [ ] **Step 2: Create `style/automation_panel.css`**

```css
.auto-panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: 320px;
    background: var(--panel-bg, #1e1e1e);
    color: var(--panel-fg, #ddd);
    border-left: 1px solid var(--panel-border, #333);
    transform: translateX(100%);
    transition: transform 180ms ease-out;
    display: flex;
    flex-direction: column;
    z-index: 50;
}
.auto-panel.open {
    transform: translateX(0);
}
.auto-panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    border-bottom: 1px solid var(--panel-border, #333);
}
.auto-panel-header h2 { margin: 0; font-size: 14px; font-weight: 600; }
.auto-panel-close { background: none; color: inherit; border: 0; font-size: 18px; cursor: pointer; }
.auto-panel-controls { display: flex; gap: 8px; padding: 8px 12px; }
.auto-panel-controls button { padding: 4px 10px; cursor: pointer; }
.auto-panel-error {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 12px;
    background: rgba(220, 50, 50, 0.18);
    border-bottom: 1px solid rgba(220, 50, 50, 0.4);
}
.auto-panel-body { flex: 1; overflow-y: auto; padding: 8px 12px; }
.auto-panel-empty { color: var(--panel-fg-muted, #888); font-style: italic; }
.auto-panel-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
```

- [ ] **Step 3: Wire the CSS file**

Open the existing CSS index (likely `style/style.css` or `index.html`). Match the pattern used for `left_drawer.css` and add an `@import` or `<link>` for `automation_panel.css`. If unsure, run:

`rg -n 'left_drawer|@import' style/ index.html 2>/dev/null`

…and follow the pattern verbatim.

- [ ] **Step 4: Verify**

Run: `cargo check --lib`
Expected: clean.

---

### Task 12: `<RuleRow>` and `<TargetPicker>` — inline editor

**Files:**
- Create: `src/components/automation_panel/rule_row.rs`
- Create: `src/components/automation_panel/target_picker.rs`
- Modify: `src/components/automation_panel/mod.rs`

- [ ] **Step 1: Register submodules in `mod.rs`**

At the top of `src/components/automation_panel/mod.rs`:

```rust
mod rule_row;
mod target_picker;

use rule_row::RuleRow;
```

- [ ] **Step 2: Create `src/components/automation_panel/target_picker.rs`**

```rust
use leptos::prelude::*;

use crate::automation::types::AutomationTarget;

#[component]
pub fn TargetPicker(
    target: Signal<AutomationTarget>,
    set_target: Callback<AutomationTarget>,
) -> impl IntoView {
    let is_cell = move || matches!(target.get(), AutomationTarget::Cell { .. });

    let select_cell = move |_| {
        if !matches!(target.get_untracked(), AutomationTarget::Cell { .. }) {
            set_target.run(AutomationTarget::Cell { sheet: 0, row: 1, column: 1 });
        }
    };
    let select_named = move |_| {
        if !matches!(target.get_untracked(), AutomationTarget::NamedRange(_)) {
            set_target.run(AutomationTarget::NamedRange(String::new()));
        }
    };

    // A1 input for Cell variant. Format: "Sheet1!B5" or "B5".
    // For now: parse as 1-indexed Excel-style; reject if it doesn't look right.
    // The exact parser entry point used by the formula bar lives in
    // src/input/formula_input.rs — reuse it rather than rolling a fresh one.
    let on_cell_input = move |ev: web_sys::Event| {
        let s = event_target_value(&ev);
        if let Some(parsed) = parse_a1(&s) {
            set_target.run(AutomationTarget::Cell { sheet: 0, row: parsed.0, column: parsed.1 });
        }
    };

    let on_name_input = move |ev: web_sys::Event| {
        let s = event_target_value(&ev);
        set_target.run(AutomationTarget::NamedRange(s));
    };

    let cell_display = move || match target.get() {
        AutomationTarget::Cell { row, column, .. } => to_a1(row, column),
        AutomationTarget::NamedRange(_) => String::new(),
    };
    let named_display = move || match target.get() {
        AutomationTarget::NamedRange(n) => n,
        AutomationTarget::Cell { .. } => String::new(),
    };

    view! {
        <div class="rule-target">
            <label>
                <input type="radio" prop:checked=is_cell on:change=select_cell />
                "Cell"
            </label>
            <label>
                <input type="radio" prop:checked=move || !is_cell() on:change=select_named />
                "Named range"
            </label>
            <Show
                when=is_cell
                fallback=move || view! {
                    <input
                        class="rule-named-input"
                        type="text"
                        placeholder="name"
                        prop:value=named_display
                        on:input=on_name_input
                    />
                }
            >
                <input
                    class="rule-cell-input"
                    type="text"
                    placeholder="B5"
                    prop:value=cell_display
                    on:input=on_cell_input
                />
            </Show>
        </div>
    }
}

// Lightweight A1 helpers. The full parser in src/input/formula_input.rs is
// preferable; swap to it once you've located the public entry point.
fn parse_a1(s: &str) -> Option<(i32, i32)> {
    let s = s.trim().to_ascii_uppercase();
    let (letters, digits): (String, String) = s.chars().partition(|c| c.is_ascii_alphabetic());
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let row: i32 = digits.parse().ok()?;
    let mut col: i32 = 0;
    for c in letters.chars() {
        col = col * 26 + (c as i32 - 'A' as i32 + 1);
    }
    Some((row, col))
}

fn to_a1(row: i32, column: i32) -> String {
    let mut c = column;
    let mut letters = String::new();
    while c > 0 {
        let rem = ((c - 1) % 26) as u8;
        letters.insert(0, (b'A' + rem) as char);
        c = (c - 1) / 26;
    }
    format!("{letters}{row}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a1_round_trip() {
        assert_eq!(parse_a1("B5"), Some((5, 2)));
        assert_eq!(parse_a1("AA10"), Some((10, 27)));
        assert_eq!(to_a1(5, 2), "B5");
        assert_eq!(to_a1(10, 27), "AA10");
    }
    #[test]
    fn a1_rejects_garbage() {
        assert_eq!(parse_a1(""), None);
        assert_eq!(parse_a1("B"), None);
        assert_eq!(parse_a1("5"), None);
    }
}
```

- [ ] **Step 3: Create `src/components/automation_panel/rule_row.rs`**

```rust
use leptos::prelude::*;

use crate::automation::types::{AutomationRule, AutomationTarget};
use crate::components::automation_panel::target_picker::TargetPicker;
use crate::state::WorkbookState;

#[component]
pub fn RuleRow(rule_id: u64) -> impl IntoView {
    let state = expect_context::<WorkbookState>();

    // Derived signal for this rule's current data.
    let rule = move || {
        state
            .automation_rules
            .with(|rs| rs.iter().find(|r| r.id == rule_id).cloned())
    };

    let update_this = move |f: Box<dyn FnOnce(&mut AutomationRule)>| {
        state.automation_rules.update(|rs| {
            if let Some(r) = rs.iter_mut().find(|r| r.id == rule_id) {
                f(r);
            }
        });
    };

    let on_label_input = move |ev: web_sys::Event| {
        let s = event_target_value(&ev);
        update_this(Box::new(move |r| r.label = s));
    };
    let on_step_input = move |ev: web_sys::Event| {
        if let Ok(v) = event_target_value(&ev).parse::<f64>() {
            update_this(Box::new(move |r| r.step = v));
        }
    };
    let on_interval_input = move |ev: web_sys::Event| {
        if let Ok(v) = event_target_value(&ev).parse::<u32>() {
            let clamped = v.max(100);
            update_this(Box::new(move |r| r.interval_ms = clamped));
        }
    };
    let on_enabled_toggle = move |ev: web_sys::Event| {
        let checked = event_target_checked(&ev);
        update_this(Box::new(move |r| r.enabled = checked));
    };
    let on_delete = move |_| {
        state.automation_rules.update(|rs| rs.retain(|r| r.id != rule_id));
    };
    let set_target = Callback::new(move |t: AutomationTarget| {
        update_this(Box::new(move |r| r.target = t));
    });

    let target_sig = Signal::derive(move || {
        rule().map(|r| r.target).unwrap_or(AutomationTarget::Cell { sheet: 0, row: 1, column: 1 })
    });

    view! {
        <li class="rule-row">
            <Show when=move || rule().is_some() fallback=|| view! { <div /> }>
                {move || {
                    let r = rule().unwrap();
                    view! {
                        <div class="rule-row-head">
                            <label>
                                <input type="checkbox" prop:checked=r.enabled on:change=on_enabled_toggle />
                            </label>
                            <input
                                class="rule-label-input"
                                type="text"
                                placeholder="label"
                                prop:value=r.label.clone()
                                on:input=on_label_input
                            />
                            <button class="rule-delete" on:click=on_delete title="Delete">"🗑"</button>
                        </div>
                        <TargetPicker target=target_sig set_target=set_target />
                        <div class="rule-numbers">
                            <label>
                                "Step:" <input type="number" step="any"
                                    prop:value=r.step
                                    on:input=on_step_input
                                />
                            </label>
                            <label>
                                "Every:" <input type="number" min="100" step="50"
                                    prop:value=r.interval_ms
                                    on:input=on_interval_input
                                /> " ms"
                            </label>
                        </div>
                    }
                }}
            </Show>
        </li>
    }
}

// Small helper used above; if your codebase already has one, prefer it.
fn event_target_checked(ev: &web_sys::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .map(|el| el.checked())
        .unwrap_or(false)
}
```

- [ ] **Step 4: Render `<For>` inside the shell**

Edit `src/components/automation_panel/mod.rs` — replace the empty `<ul class="auto-panel-list">` with:

```rust
<ul class="auto-panel-list">
    <For
        each=move || state.automation_rules.get()
        key=|r| r.id
        let:rule
    >
        <RuleRow rule_id=rule.id />
    </For>
</ul>
```

- [ ] **Step 5: Add row CSS to `style/automation_panel.css`**

```css
.rule-row {
    border: 1px solid var(--panel-border, #333);
    border-radius: 4px;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
}
.rule-row-head { display: flex; align-items: center; gap: 6px; }
.rule-label-input { flex: 1; }
.rule-target { display: flex; align-items: center; gap: 8px; }
.rule-target label { display: flex; align-items: center; gap: 4px; }
.rule-target input[type="text"] { flex: 1; }
.rule-numbers { display: flex; gap: 12px; }
.rule-numbers input { width: 80px; }
.rule-delete { background: none; border: 0; cursor: pointer; }
```

- [ ] **Step 6: Verify**

Run: `cargo check --lib && cargo test --lib automation`
Expected: clean compile; all `automation::*` tests still pass.

---

### Task 13: Persistence hookup

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Add imports near the top of `app.rs`**

```rust
use crate::automation;
```

- [ ] **Step 2: Load rules on workbook bootstrap**

Find the line `wb_state.current_uuid.set(Some(uuid));` near the top of `App()`. Immediately after it, add:

```rust
    {
        let rules = automation::storage::load(&uuid);
        wb_state.automation_rules.set(rules);
    }
```

- [ ] **Step 3: Save rules on every mutation**

After the existing auto-save `Effect`, add a second `Effect` that mirrors rule changes to localStorage:

```rust
    Effect::new(move |_| {
        let Some(uuid) = wb_state.current_uuid.get() else { return; };
        // Read reactively so this Effect re-runs on every rule edit.
        let rules = wb_state.automation_rules.get();
        automation::storage::save(&uuid, &rules);
    });
```

- [ ] **Step 4: Reset transient state on reload**

`automation_running` / `automation_panel_open` / `automation_error` already default to `false / false / None` from `WorkbookState::new`. No extra work — verify by reading the file.

- [ ] **Step 5: Verify**

Run: `cargo check --lib`
Expected: clean.

---

### Task 14: Toolbar trigger

**Files:**
- Modify: `src/components/toolbar/mod.rs`

- [ ] **Step 1: Add the button to the toolbar view**

Inside the `view! { <div class="tb"> … </div> }` block of `Toolbar()`, after the last existing section (look for an appropriate spot near other utility buttons), add:

```rust
        <button
            class="tb-btn"
            on:click=move |_| state.automation_panel_open.update(|v| *v = !*v)
            title="Automation"
        >
            "⚙ Auto"
        </button>
```

`state` already binds `WorkbookState` at the top of `Toolbar()`.

- [ ] **Step 2: Verify**

Run: `cargo check --lib`
Expected: clean.

---

### Task 15: Mount `<AutomationPanel>` in the layout

**Files:**
- Modify: `src/app.rs`

- [ ] **Step 1: Import the component**

Near the top of `src/app.rs`, with the other `use crate::components::…;` lines:

```rust
use crate::components::automation_panel::AutomationPanel;
```

- [ ] **Step 2: Mount it in the root `view!`**

Replace:

```rust
view! {
    <div id="app">
        <LeftDrawer />
        <Workbook />
    </div>
}
```

with:

```rust
view! {
    <div id="app">
        <LeftDrawer />
        <Workbook />
        <AutomationPanel />
    </div>
}
```

- [ ] **Step 3: Verify**

Run: `cargo check --lib && cargo test --lib`
Expected: clean compile; all 14 `automation::*` tests pass.

---

### Task 16: Manual smoke test & finalization

**Files:** none — verification only.

- [ ] **Step 1: Start the dev server**

Run: `trunk serve --open` (or whatever the project's run command is — check `README.md` or `Cargo.toml`).

- [ ] **Step 2: Smoke-test the golden path**

In the browser:
1. Click the `⚙ Auto` toolbar button — panel slides in from the right.
2. Click `+ Rule` — a row appears with default values.
3. Set the target to `B1` and type `5` into B1 in the grid.
4. Click `▶ Start` — watch B1 climb by 1 every 1000 ms in the canvas.
5. Click `⏸ Stop` — B1 stops.

- [ ] **Step 3: Smoke-test the error path**

1. Set the target to `B2` and type `=1+2` into B2 (a formula).
2. Click `▶ Start`.
3. Expected: the error banner appears with `"<label>: cell holds a formula; refusing to overwrite"`, the button reverts to `▶ Start`, and no automation runs.

- [ ] **Step 4: Smoke-test persistence**

1. Create a rule, do not press Start.
2. Reload the page.
3. Expected: panel closed (must re-open via toolbar), rule list still present with the same values.

- [ ] **Step 5: Smoke-test workbook switching**

1. Add rules to workbook A.
2. In the left drawer, create a new workbook B; switch to it.
3. Expected: rule list is empty.
4. Switch back to A.
5. Expected: A's rules are restored.

- [ ] **Step 6: Final type/test pass**

Run: `cargo check --lib`
Expected: clean, no warnings introduced.

Run: `cargo test --lib`
Expected: all tests pass, including the 14 `automation::*` tests.

If `clippy` is part of the project's CI (it is — see `.github/workflows/rustycalc.yml`):

Run: `cargo clippy --lib -- -D warnings`
Expected: clean.

- [ ] **Step 7: Report completion to the user**

Summarize:
- Files added / modified
- Test counts
- Smoke-test outcomes
- Any deviations from the plan (e.g. "the named-range parser entry point was X, not Y as the plan guessed")

Do NOT commit. The user manages commits.

---

## Self-review notes

Spec coverage checked:
- Data model — Task 1
- State placement — Task 2
- Persistence — Tasks 3, 13 (sidecar approach documented as a spec divergence)
- UI mount + slide animation — Tasks 11, 15
- Toolbar trigger — Task 14
- Rule row inline editor — Task 12
- Target picker (cell + named range) — Task 12
- Ticker engine — Tasks 7, 8
- Ticker reactive wrapper — Task 9
- Stop-on-failure & error banner — Tasks 9, 11
- Testing — every pure-logic module ships with `#[cfg(test)] mod tests`; manual smoke covers the UI layer
- Cleanup on workbook switch — Task 13 (load runs on `current_uuid` change in app bootstrap)

Type/method-name consistency:
- `set_user_input(sheet: u32, row: i32, column: i32, value: &str)` — used identically in Tasks 6, 7
- `ResolvedAddress` — defined in Task 4, used in Tasks 6, 7
- `TickError` variants — exhaustively used by their constructor sites and tests
- `state.automation_rules` — same accessor pattern in every task (`.get()`, `.update()`, `.with()`)
- `WorkbookId` — re-uses the existing type from `src/storage.rs`

Placeholder scan: no "TBD", no "implement later", no "add appropriate error handling". A small number of explicit "verify against the source" notes remain in Tasks 5, 6, 11, 12 — these are deliberate (the engineer must use the project's existing parser/accessor entry points, not reinvent them), and each has an exact `rg` command to find the right symbol.
