# Performance: Avoiding Double Evaluation

IronCalc's `UserModel` calls `evaluate()` internally after many mutations. If the caller also evaluates, performance halves in formula-heavy spreadsheets.

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

`src/model/frontend_model.rs` provides two helpers. Both pause evaluation before `f`, resume after, and optionally evaluate once — never more. Neither emits events; the caller calls `state.emit_event(...)` after the helper returns.

```rust
pub fn mutate(model, evaluate: EvaluationMode, f: impl FnOnce(&mut UserModel))
pub fn try_mutate<E>(model, evaluate, f: impl FnOnce(&mut UserModel) -> Result<(), E>) -> Result<(), E>
```

Use `mutate` when the closure can't fail, `try_mutate` when it can.

**Import:** `use crate::model::{mutate, try_mutate, EvaluationMode};`

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

```rust
// Cell edit — fallible, formulas must recalc
try_mutate(model, EvaluationMode::Immediate, |m| -> Result<(), EditError> {
    m.set_user_input(sheet, row, col, value).map_err(EditError::Engine)
})?;
state.emit_event(SpreadsheetEvent::Content(ContentEvent::CellChanged { .. }));

// Formatting — fallible, no recalc
try_mutate(model, EvaluationMode::Deferred, |m| -> Result<(), FormatError> {
    m.update_range_style(&area, path, value).map_err(FormatError::Engine)
})?;

// Navigation — infallible, no recalc
mutate(model, EvaluationMode::Deferred, |m| { m.nav_arrow(dir); });
```

## Performance Impact

Pause/resume bracketing avoids the second `evaluate()` per mutation. The win scales with formula-graph fanout: cells with many dependents, deep chains, or rapid typing benefit most. Lightly-formula'd workbooks see negligible difference.

To measure on a real workbook:

- **Render time**: `src/components/worksheet.rs` brackets the canvas paint with `console::time_with_label("render")` / `console::time_end_with_label("render")`. Open DevTools → Performance and look for the `render` measure.
- **Perf panel** (`src/components/perf_panel.rs`): toggleable in-app overlay backed by `AppState::perf` (`PerfTimings`); shows commit→render timestamps for the last action. Enable via `app.show_perf_panel.set(true)`.
- **Event timing**: see "Debugging Evaluation Timing" below.

## Guidelines

- Never call `m.evaluate()` inside the closure — the helper does it.
- Always `state.emit_event(...)` after the helper returns — neither helper notifies subscribers.

## Debugging Evaluation Timing

Debug event logging is wired into `emit_events()` but currently commented out. Uncomment the `leptos::logging::log!` call in `state.rs` to see per-event timestamps. Large gaps (>100ms) between events suggest double evaluation — check that `mutate` is being used rather than bare `model.update_value` + `m.evaluate()`.

## Implementation Details

IronCalc's `pause_evaluation()` increments an internal counter; `resume_evaluation()` decrements it. Internal `evaluate()` calls are no-ops when the counter > 0. The final `evaluate()` after `resume_evaluation()` does the actual work. Pausing and batching doesn't change results — only when the work happens.
