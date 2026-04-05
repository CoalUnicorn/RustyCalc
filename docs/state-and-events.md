# State and Events

`WorkbookState` holds all transient UI state — nothing persisted, nothing in the model. `events.rs` defines the typed events that components emit and subscribe to.

These two files are tightly coupled: every mutation that should trigger a UI update ends with an `emit_event()` call.

---

## WorkbookState

`WorkbookState` is a `Copy` struct provided via Leptos context. All fields are public within the crate and accessed directly — no getter methods.

```rust
let state = expect_context::<WorkbookState>();

// Reading (reactive — registers a dependency):
let editing = state.editing_cell.get();

// Reading (non-reactive — safe inside event handlers):
let uuid = state.current_uuid.get_untracked();

// Writing:
state.drag.set(DragState::Idle);
state.editing_cell.update(|c| {
    if let Some(e) = c { e.text = new_text; }
});
```

### Split\<T\>

Every field is a `Split<T>` — a thin wrapper around a Leptos `(ReadSignal<T>, WriteSignal<T>)` pair. It's `Copy` for any `T: Clone + Send + Sync + 'static`, even non-Copy types, because signal handles are arena IDs.

| Method | What it does |
|--------|-------------|
| `.get()` | Reactive read. Use inside `move \|\|` closures, effects, memos. |
| `.get_untracked()` | Non-reactive read. Use in event handlers and callbacks. |
| `.with(f)` | Borrow without cloning (reactive). |
| `.with_untracked(f)` | Borrow without cloning (non-reactive). |
| `.set(v)` | Replace value. Always notifies subscribers. |
| `.update(f)` | Mutate in place. |
| `.read()` | Returns `ReadSignal<T>` — pass to read-only child components. |
| `.write()` | Returns `WriteSignal<T>` — pass to mutating child components. |

**`.get()` vs `.get_untracked()`:** Use `.get()` when the enclosing closure should re-run when the signal changes. Use `.get_untracked()` in event handlers — you want the current value but not a subscription.

```rust
// Wrong: registers a reactive dependency inside an event handler.
// The handler re-fires on every editing_cell change, not just on clicks.
let on_click = move |_| {
    if let Some(edit) = state.editing_cell.get() { ... }
};

// Right:
let on_click = move |_| {
    if let Some(edit) = state.editing_cell.get_untracked() { ... }
};
```

### Theme methods are different

`state.theme` holds the user's *preference* (Auto/Light/Dark). Auto needs to be resolved against the system dark-mode setting. Two methods on `WorkbookState` do that:

- `state.get_theme()` — reactive, resolves Auto → Light or Dark
- `state.get_theme_untracked()` — non-reactive version

Don't call `state.theme.get()` directly in components — it won't resolve Auto correctly.

---

## EventBus

`state.events` has one `RwSignal<Vec<EventType>>` per category:

```
state.events.content    →  Vec<ContentEvent>
state.events.format     →  Vec<FormatEvent>
state.events.navigation →  Vec<NavigationEvent>
state.events.structure  →  Vec<StructureEvent>
state.events.mode       →  Vec<ModeEvent>
state.events.theme      →  Vec<ThemeEvent>
```

Each `emit_event()` call **replaces** all six signals. The previous action's events are gone. Components subscribing to `state.events.navigation.get()` see only the events from the most recent emit.

This is intentional. `state.events` is not a history buffer — it's a snapshot of what just happened. Components read it to decide whether to update.

### Emitting

```rust
// Single event:
state.emit_event(SpreadsheetEvent::Structure(
    StructureEvent::WorksheetAdded { sheet: 2, name: "Sheet3".into() },
));

// Multiple events in one call (one signal update, preferred):
state.emit_events([
    SpreadsheetEvent::Content(ContentEvent::RangeChanged { sheet, start_row, start_col, end_row, end_col }),
    SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged { address }),
]);
```

Two separate `emit_event()` calls work but fire all six signals twice. Use `emit_events()` when one user action produces more than one event.

`state.request_redraw()` is shorthand for `emit_event(Content(GenericChange))`. It's used when a canvas repaint is needed without a specific event to describe it (e.g. viewport resize).

### Subscribing

Read the category signal inside a reactive closure. The closure re-runs whenever that category gets new events.

```rust
// Re-runs on every structure event:
let sheet_list = move || {
    let _ = state.events.structure.get(); // subscribe — value not used
    model.with_value(|m| m.get_worksheets_properties())
};

// Subscribe to two categories:
let cell_address = move || {
    let _ = state.events.content.get();
    let _ = state.events.navigation.get();
    model.with_value(|m| m.active_cell())
};
```

Don't subscribe to more categories than needed. A component subscribed to `content` re-runs on every cell edit. If it only cares about sheet switches, subscribe to `structure` instead.

Checking which specific events arrived:
```rust
let has_layout_change = move || {
    state.events.format.get()
        .iter()
        .any(|e| matches!(e, FormatEvent::LayoutChanged { .. }))
};
```

---

## Adding a new event variant

Example: tracking when a sheet is frozen.

**1. Add the variant in `events.rs`:**

```rust
// Inside StructureEvent:
/// Row/column freeze changed on a sheet.
WorksheetFreezeChanged { sheet: u32, rows: i32, cols: i32 },
```

**2. Update `affected_sheet()`:**

```rust
StructureEvent::WorksheetFreezeChanged { sheet, .. } => Some(*sheet),
```

**3. Update `dbg_description()`:**

```rust
StructureEvent::WorksheetFreezeChanged { sheet, rows, cols } => {
    format!("Structure::Freeze S{sheet} rows={rows} cols={cols}")
}
```

**4. Emit it from the action handler:**

```rust
state.emit_event(SpreadsheetEvent::Structure(
    StructureEvent::WorksheetFreezeChanged { sheet: sheet_idx, rows: 1, cols: 0 },
));
```

The compiler flags every exhaustive `match` on `StructureEvent` that doesn't cover the new variant. Follow the errors.

### Adding a new category

This is rare — most changes fit the existing six. If you need one:

1. Add an enum in `events.rs`.
2. Add a variant to `SpreadsheetEvent`.
3. Add a `RwSignal<Vec<NewEvent>>` field to `EventBus` and initialize it in `EventBus::new()`.
4. Add the dispatch arm in `WorkbookState::emit_events()`.
5. Add the `dbg_description()` dispatch in `SpreadsheetEvent::dbg_description()`.

---

## Fields reference

| Field | Type | Purpose |
|-------|------|---------|
| `editing_cell` | `Split<Option<EditingCell>>` | Active in-progress cell edit. `None` when not editing. |
| `drag` | `Split<DragState>` | Current mouse-drag mode: selecting, resizing, autofill, pointing. |
| `current_uuid` | `Split<Option<String>>` | UUID of the loaded workbook — used for auto-save. |
| `theme` | `Split<Theme>` | User's theme preference (Auto/Light/Dark). Read via `get_theme()`, not `.theme.get()`. |
| `show_perf_panel` | `Split<bool>` | Whether the performance panel overlay is visible. |
| `context_menu` | `Split<Option<ContextMenuState>>` | Active right-click menu position and header target. |
| `point_range` | `Split<Option<SheetRect>>` | Cell range highlighted during formula point-mode entry. |
| `point_ref_span` | `Split<Option<(usize, usize)>>` | Byte range in `editing_cell.text` for the current point-mode reference. |
| `formula_input_ref` | `NodeRef<Input>` | DOM ref to the formula bar `<input>` — used to read cursor position for point-mode. |
| `recent_colors` | `Split<Vec<CssColor>>` | Recently used colors (max 16), persisted to localStorage. |
| `perf` | `PerfTimings` | Timestamps for the commit → render pipeline, displayed in the perf panel. |
| `events` | `EventBus` | Per-category event signals. |

### DragState

```
Idle                          — no drag active
Selecting                     — mouse held for range selection
Extending { to_row, to_col }  — autofill handle drag
ResizingCol { col, x }        — column header resize
ResizingRow { row, y }        — row header resize
Pointing                      — dragging to extend a formula point-mode range
```

At most one is active at a time. The enum makes illegal combinations unrepresentable.

---

## Event category guide

| Category | When to use |
|----------|-------------|
| `Content` | Cell values, formulas, calculation results changed |
| `Format` | Visual styling changed: fonts, colors, column widths, row heights |
| `Structure` | Sheet added/deleted/renamed/hidden, rows or columns inserted/deleted |
| `Navigation` | Selection moved, sheet switched, viewport scrolled, edit started/ended |
| `Mode` | Drag mode changed, context menu toggled, point mode entered/exited |
| `Theme` | Light/dark theme toggled, color palette updated |

See `adding-actions.md` for the action pipeline that produces these events.
