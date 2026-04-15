# State and Events

Two context structs carry reactive UI state. `AppState` holds application-level signals (sidebar, theme, group collapse) that are independent of which workbook is loaded. `WorkbookState` holds spreadsheet editing state that is scoped to the active session. Both share the same `EventBus` instance, constructed once in `App` and passed to each.

Every model mutation that should trigger a UI update ends with `emit_event()`.

---

## AppState

`Copy` struct provided via Leptos context. Holds signals that survive workbook switches.

```rust
let app = expect_context::<AppState>();

app.sidebar_open.set(true);
app.bump_registry();          // increments registry_version; redraws left drawer
app.toggle_theme();
```

Theme resolution: `app.theme` holds the user's preference (`Auto/Light/Dark`). Call `app.get_theme()` to resolve `Auto` against the system setting. Don't read `app.theme.get()` directly in rendering code.

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

### Bridging a Split signal to component props

Some components (e.g. `ContextMenu`) require a `(ReadSignal<bool>, WriteSignal<bool>)` pair,
but the source of truth is a `Split<Option<T>>` on `WorkbookState`. Use two `Effect`s to sync
in both directions:

```rust
let (menu_open, set_menu_open) = signal(false);
let (menu_pos,  set_menu_pos)  = signal((0i32, 0i32));

// Effect 1 — state → local signals (runs when the external signal changes)
Effect::new(move |_| {
    match state.context_menu.get() {   // reactive read
        Some(ctx) => {
            set_menu_pos.set((ctx.x, ctx.y));
            set_menu_open.set(true);
        }
        None => set_menu_open.set(false),
    }
});

// Effect 2 — local close → clear state (runs when menu_open changes)
Effect::new(move |prev: Option<bool>| {
    let is_open = menu_open.get();          // reactive read
    if prev == Some(true) && !is_open {
        state.context_menu.set(None);       // non-reactive write
    }
    is_open  // returned value becomes `prev` on the next run
});
```

**Why the cycle terminates:** each Effect has exactly one reactive read and writes
only to things it doesn't read. When Effect 2 clears `state.context_menu`, Effect 1
fires and calls `set_menu_open.set(false)` — a no-op since it's already false.
Leptos skips notifying downstream subscribers on a no-op write, so nothing loops.

**Why `prev: Option<bool>` instead of just comparing to false:** on the first run
`prev` is `None` (the signal was just created). Without the `prev == Some(true)` guard,
closing logic would fire spuriously at mount time before the menu ever opened.

This pattern applies whenever a canvas mouse handler or other non-component code
writes to a `Split` field, and a component needs to consume it via
`(ReadSignal, WriteSignal)` props.

### Theme methods

Theme preference lives on `AppState`, not `WorkbookState`. `app.theme` stores the raw preference (`Auto/Light/Dark`); `app.get_theme()` resolves `Auto` against the system setting. Use `app.get_theme()` in rendering code, not `app.theme.get()` directly.

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

### WorkbookState fields

| Field | Type | Purpose |
|-------|------|---------|
| `editing_cell` | `Split<Option<EditingCell>>` | Active in-progress cell edit. `None` when not editing. |
| `drag` | `Split<DragState>` | Current mouse-drag mode: selecting, resizing, autofill, pointing. |
| `current_uuid` | `Split<Option<WorkbookId>>` | ID of the loaded workbook — used for auto-save and storage lookups. |
| `context_menu` | `Split<Option<ContextMenuState>>` | Active right-click menu position and header target. |
| `formula_input_ref` | `NodeRef<Input>` | DOM ref to the formula bar `<input>` - used to read cursor position for point-mode. |
| `recent_colors` | `Split<Vec<CssColor>>` | Recently used colors (max 16), persisted to localStorage. |
| `status` | `Split<Option<StatusMessage>>` | Current status bar message. `None` clears the bar; `Some(StatusMessage::Error(msg))` shows an error. Set by `execute()` on every action (clears on `Ok`, sets on `Err`) and by direct sheet/workbook mutations. |
| `events` | `EventBus` | Per-category event signals. |

### AppState fields

| Field | Type | Purpose |
|-------|------|---------|
| `theme` | `Split<Theme>` | User's theme preference. Read via `app.get_theme()`, not `.theme.get()`. |
| `sidebar_open` | `Split<bool>` | Left drawer visibility. |
| `collapsed_groups` | `Split<Vec<String>>` | Group labels currently collapsed in the left drawer. |
| `show_perf_panel` | `Split<bool>` | Whether the performance panel overlay is visible. |
| `perf` | `PerfTimings` | Timestamps for the commit → render pipeline. |
| `registry_version` | `RwSignal<u64>` | Bumped on workbook CRUD. Left drawer subscribes to this; nothing else should. |

### DragState

```
Idle                                        - no drag active
Selecting                                   - mouse held for range selection
Extending { to_row, to_col }                - autofill handle drag
ResizingCol { col, x }                      - column header resize
ResizingRow { row, y }                      - row header resize
Pointing { range: CellArea,
           ref_span: RefSpan }              - formula point-mode: highlighted range
                                              + byte span in formula text being replaced
                                              (RefSpan is {start, end: usize} from coord.rs)
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
