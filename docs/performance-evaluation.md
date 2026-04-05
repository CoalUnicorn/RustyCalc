# Performance: Avoiding Double Evaluation

This document explains the evaluation performance optimization in RustyCalc and how to use the `mutate` helper function correctly.

## The Problem: Double Evaluation

IronCalc's `UserModel` has an internal evaluation system. Many methods call `evaluate()` internally after making changes. However, our UI often needs to call `evaluate()` again to ensure consistency. This creates a **double evaluation problem** that can halve performance in formula-heavy spreadsheets.

```rust
// PERFORMANCE PROBLEM: Double evaluation
model.update_value(|m| {
    m.set_cell_value(sheet, row, col, value);  // Calls evaluate() internally
    m.evaluate();  // Called again! Doubles the work
});
```

## The Solution: pause_evaluation/resume_evaluation

IronCalc provides `pause_evaluation()` and `resume_evaluation()` methods specifically for this case. Pausing evaluation before mutations prevents the internal calls from doing work, then we evaluate once at the end.

```rust
// PERFORMANCE OPTIMIZED: Single evaluation  
model.update_value(|m| {
    m.pause_evaluation();           // Prevent internal evaluate() calls
    m.set_cell_value(sheet, row, col, value);  // No evaluation
    m.resume_evaluation();          // Re-enable evaluation
    m.evaluate();                   // Single evaluation at the end
});
```

## The mutate Helpers

`src/input/helpers.rs` provides two helpers. Both wrap pause/resume evaluation — the only difference is whether the closure can fail.

### `mutate` — infallible

Use when the closure cannot return an error (navigation, selection changes):

```rust
pub fn mutate(
    model: ModelStore,
    _state: &WorkbookState,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>),
)
```

### `try_mutate` — fallible

Use when the closure returns `Result`. The error is returned to the caller and can be propagated with `?`:

```rust
pub fn try_mutate<E>(
    model: ModelStore,
    _state: &WorkbookState,
    evaluate: EvaluationMode,
    f: impl FnOnce(&mut UserModel<'static>) -> Result<(), E>,
) -> Result<(), E>
```

Both helpers pause evaluation before calling `f`, then resume and optionally evaluate once — never more.

Neither emits events or triggers redraws. The caller is responsible for `state.emit_event(...)` after the helper returns.

**Import:** `use crate::input::helpers::{mutate, try_mutate, EvaluationMode};`

## When to Evaluate

`EvaluationMode` controls whether `evaluate()` is called after the mutation:

### EvaluationMode::Immediate
Use when mutations **may change formula results**:
- Cell value/formula changes
- Row/column insertions/deletions
- Sheet operations that affect references
- Copy/paste operations

### EvaluationMode::Deferred
Use for **pure UI state changes** that don't affect calculations:
- Navigation (arrow keys, selection changes)
- Formatting (bold, italic, colors, fonts)
- UI state (freeze panes, column widths)
- Theme changes

## Usage Examples

### Cell Edit (fallible, evaluation needed)
```rust
// try_mutate propagates the engine error back to the caller via ?
try_mutate(model, state, EvaluationMode::Immediate, |m| -> Result<(), EditError> {
    m.set_user_input(sheet, row, col, value)
        .map_err(EditError::Engine)?;
    Ok(())
})?;
state.emit_event(SpreadsheetEvent::Content(ContentEvent::CellChanged { .. }));
```

### Navigation (infallible, no evaluation)
```rust
// nav_arrow never fails — plain mutate is fine
mutate(model, state, EvaluationMode::Deferred, |m| {
    m.nav_arrow(dir);
});
state.emit_event(SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged { .. }));
```

### Formatting (fallible, no evaluation)
```rust
try_mutate(model, state, EvaluationMode::Deferred, |m| -> Result<(), FormatError> {
    let area = selection_area(m);
    m.update_range_style(&area, style_path.as_str(), value)
        .map_err(FormatError::Engine)?;
    Ok(())
})?;
state.emit_event(SpreadsheetEvent::Format(FormatEvent::RangeStyleChanged { .. }));
```

### Structure Change (fallible, evaluation needed)
```rust
// Row insertion affects formula references
try_mutate(model, state, EvaluationMode::Immediate, |m| -> Result<(), StructError> {
    m.insert_rows(sheet, row, 1)
        .map_err(StructError::Engine)?;
    Ok(())
})?;
state.emit_event(SpreadsheetEvent::Structure(StructureEvent::rows_inserted(loc)));
```

## Performance Impact

In testing with formula-heavy spreadsheets:
- **Without pause/resume**: 200ms per cell edit (double evaluation)
- **With pause/resume**: 100ms per cell edit (single evaluation)  

The optimization becomes more important as:
- Formula complexity increases
- Number of dependent cells grows
- Frequency of mutations increases (typing, rapid operations)

## Guidelines

1. Use `try_mutate` when the closure can fail; use `mutate` for infallible arms.
2. **Import:** `use crate::input::helpers::{mutate, try_mutate, EvaluationMode};`
3. Pass `EvaluationMode::Immediate` when formulas might be affected (cell writes, row/col inserts).
4. Pass `EvaluationMode::Deferred` for pure UI changes (navigation, formatting, selection).
5. Never call `m.evaluate()` manually inside either helper's closure — the helper handles it.
6. Always emit a typed event after the helper returns — neither helper triggers redraws or notifies subscribers.

## Debugging Evaluation Timing

In debug builds, every event emitted through `state.emit_event()` / `emit_events()` is logged to the browser console with a relative timestamp:

```
[EventBus] +    12.34ms  Content::GenericChange
[EventBus] +     0.12ms  Navigation::SelectionChanged row=2 col=1
[EventBus] +   142.80ms  Content::RangeChanged sheet=1 r1=3 c1=1 r2=3 c2=1
```

The delta shows time since the previous event. Large gaps (>100ms) in a tight sequence indicate double evaluation or an unpaused `evaluate()` call. Check that `mutate` is being used rather than a bare `model.update_value` + `m.evaluate()`.

## Implementation Details

The pause/resume pattern works because:
- IronCalc tracks evaluation state with internal flags
- `pause_evaluation()` increments a counter  
- `resume_evaluation()` decrements the counter
- Internal `evaluate()` calls are no-ops when counter > 0
- Final `evaluate()` after `resume_evaluation()` does the actual work

This is safe because evaluation is deterministic - pausing and batching doesn't change the final result, only when the work happens.
