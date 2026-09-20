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

Use a `dev-tools` build to measure a real workbook. Open **View → Perf**
to show the Performance inspector. The window starts open in this build.
The component is `src/components/panels/perf_panel/mod.rs`.

Select **New capture**, perform the actions, then select **Pause** or **Finish**.
Closing the window pauses capture and keeps its records. The capture selector
can show an earlier capture while another capture records new attempts.

- **Live samples** come from `AppState::perf` (`PerfTimings`). Each mutation
  sample owns its mutation result and evaluation state. Evaluation is measured,
  deferred, or not run. A missing duration is not zero.
- **Digest** reads the selected capture from `AppState::perf_store` (`PerfStore`).
  Attempt counts, render-call statistics, fetch totals, and painted-cell visits
  use the displayed attempt filter. Mutation totals and elapsed times cover the
  selected capture. Wall time includes pauses. Active time excludes pauses.
  Neither is input-to-paint latency. Painted-cell visits are not unique cells.
- **Attempts** retains committed and held attempts. Select a row to keep its
  details stable across later edits and scrolls. Attempt identity includes the
  canvas generation. A held attempt has no commit id.
- **Details** separates host reports, fingerprint changes, fetched ranges,
  repaint source ranges, pixel clips, and derived painted coverage. Each address
  row states its source and precision. Missing evidence stays explicit.
- **JSON** serializes the selected immutable attempt on demand. Capture export
  uses envelope version 1 and diagnostics schema 3. Tools also contains the
  existing SVG/PDF exports and `.icr` recording and playback controls. These
  sheet exports use the current view. `.icr` keeps schema 7. A recording baseline
  is retained once and excluded from the default digest.

Capture limits are 500 attempts, 5,000 host event summaries, 2,000 mutation
samples, five captures, and a shared 32 MiB estimated retained-byte budget.
The inspector shows the stop reason. Delete a completed capture to release its
budget. The byte estimate is not a measurement of total browser heap usage.

## Guidelines

- Never call `m.evaluate()` inside the closure — the helper does it.
- Always `state.emit_event(...)` after the helper returns — neither helper notifies subscribers.

## Debugging Evaluation Timing

Compare the mutation and evaluation states in the inspector. Each wrapper call
publishes one atomic sample. A failed mutation reports evaluation as not run.
The inspector does not subtract timestamps from different operations.

While capture runs, `EventBus` calls a borrowed batch observer before it updates
the category signals. This retains each emitted batch, including several batches
before one animation frame. Batch ids describe observed provenance. They do not
prove that a batch caused every cell in a repaint envelope.

The capture coordinator detaches the observer and sample sink on pause, stop,
limit, and worksheet cleanup. Default builds omit the capture store, observer,
sample sink, and inspector. They keep the small timing signal container, but
model wrappers do not publish diagnostic samples.

## Implementation Details

IronCalc's `pause_evaluation()` increments an internal counter; `resume_evaluation()` decrements it. Internal `evaluate()` calls are no-ops when the counter > 0. The final `evaluate()` after `resume_evaluation()` does the actual work. Pausing and batching doesn't change results — only when the work happens.
