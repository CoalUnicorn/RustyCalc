# State and Events

`WorkbookState` (transient UI state) and `events.rs` (typed events) work together:
every model mutation that should trigger a UI update ends with `emit_event()`.

---

## WorkbookState

`Copy` struct provided via Leptos context. Fields are `pub(crate)` — accessed directly, no getters.

```rust
let state = expect_context::<WorkbookState>();

// Reading (reactive - registers a dependency):
let editing = state.editing_cell.get();

// Reading (non-reactive - safe inside event handlers):
let uuid = state.current_uuid.get_untracked();

// Writing:
state.drag.set(DragState::Idle);
state.editing_cell.update(|c| {
    if let Some(e) = c { e.text = new_text; }
});
```

### Split\<T\>

Every field is a `Split<T>` - a thin wrapper around a Leptos `(ReadSignal<T>, WriteSignal<T>)` pair. It's `Copy` for any `T: Clone + Send + Sync + 'static`, even non-Copy types, because signal handles are arena IDs.

| Method | What it does |
|--------|-------------|
| `.get()` | Reactive read. Use inside `move \|\|` closures, effects, memos. |
| `.get_untracked()` | Non-reactive read. Use in event handlers and callbacks. |
| `.with(f)` | Borrow without cloning (reactive). |
| `.with_untracked(f)` | Borrow without cloning (non-reactive). |
| `.set(v)` | Replace value. Always notifies subscribers. |
| `.update(f)` | Mutate in place. |
| `.read()` | Returns `ReadSignal<T>` - pass to read-only child components. |
| `.write()` | Returns `WriteSignal<T>` - pass to mutating child components. |

**`.get()` vs `.get_untracked()`:** Use `.get()` when the enclosing closure should re-run when the signal changes. Use `.get_untracked()` in event handlers - you want the current value but not a subscription.

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

- `state.get_theme()` - reactive, resolves Auto -> Light or Dark
- `state.get_theme_untracked()` - non-reactive version

Don't call `state.theme.get()` directly in components - it won't resolve Auto correctly.

---

## EventBus

`state.events` has one `RwSignal<Vec<EventType>>` per category:

```
state.events.content    ->  Vec<ContentEvent>
state.events.format     ->  Vec<FormatEvent>
state.events.navigation ->  Vec<NavigationEvent>
state.events.structure  ->  Vec<StructureEvent>
state.events.theme      ->  Vec<ThemeEvent>
```

Each `emit_event()` call **replaces** all five signals — it's a snapshot of the most recent action, not a history buffer.

### Emitting

```rust
// Single event:
state.emit_event(SpreadsheetEvent::Structure(
    StructureEvent::WorksheetAdded { sheet: 2, name: "Sheet3".into() },
));

// Multiple events in one call (one signal update, preferred):
state.emit_events([
    SpreadsheetEvent::Content(ContentEvent::RangeChanged { sheet_area }),
    SpreadsheetEvent::Navigation(NavigationEvent::SelectionChanged { address }),
]);
```

Two separate `emit_event()` calls work but fire all five signals twice. Use `emit_events()` when one user action produces more than one event.

For a canvas repaint with no specific event, emit `Content(GenericChange)` directly.

### Subscribing

Read the category signal inside a reactive closure. The closure re-runs whenever that category gets new events.

```rust
// Re-runs on every structure event:
let sheet_list = move || {
    let _ = state.events.structure.get(); // subscribe - value not used
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
FreezeChanged { sheet: u32, frozen_rows: i32, frozen_cols: i32 },
```

**2. Emit it from the action handler:**

```rust
state.emit_event(SpreadsheetEvent::Structure(
    StructureEvent::FreezeChanged { sheet: sheet_idx, frozen_rows: 1, frozen_cols: 0 },
));
```

The compiler flags every exhaustive `match` on `StructureEvent` that doesn't cover the new variant. Follow the errors.

### Adding a new category

Rare — most changes fit the existing five. If you need one:

1. Add an enum in `events.rs`.
2. Add a variant to `SpreadsheetEvent`.
3. Add a `RwSignal<Vec<NewEvent>>` field to `EventBus` and initialize it in `EventBus::new()`.
4. Add the dispatch arm in `WorkbookState::emit_events()`.

---

## Fields reference

| Field | Type | Purpose |
|-------|------|---------|
| `editing_cell` | `Split<Option<EditingCell>>` | Active in-progress cell edit. `None` when not editing. |
| `drag` | `Split<DragState>` | Current mouse-drag mode: selecting, resizing, autofill, pointing. |
| `current_uuid` | `Split<Option<String>>` | UUID of the loaded workbook - used for auto-save. |
| `theme` | `Split<Theme>` | User's theme preference (Auto/Light/Dark). Read via `get_theme()`, not `.theme.get()`. |
| `show_perf_panel` | `Split<bool>` | Whether the performance panel overlay is visible. |
| `context_menu` | `Split<Option<ContextMenuState>>` | Active right-click menu position and header target. |
| `formula_input_ref` | `NodeRef<Input>` | DOM ref to the formula bar `<input>` - used to read cursor position for point-mode. |
| `recent_colors` | `Split<Vec<CssColor>>` | Recently used colors (max 16), persisted to localStorage. |
| `perf` | `PerfTimings` | Timestamps for the commit -> render pipeline, displayed in the perf panel. |
| `events` | `EventBus` | Per-category event signals. |

### DragState

```
Idle                                        - no drag active
Selecting                                   - mouse held for range selection
Extending { to_row, to_col }                - autofill handle drag
ResizingCol { col, x }                      - column header resize
ResizingRow { row, y }                      - row header resize
Pointing { range: CellArea,
           ref_span: (usize, usize) }       - formula point-mode: highlighted range
                                              + byte span in formula text being replaced
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
| `Theme` | Light/dark theme toggled, color palette updated |

See `adding-actions.md` for the action pipeline that produces these events.
