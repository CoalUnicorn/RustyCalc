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
app.toggle_light_dark();      // flips between Light and Dark
app.set_theme(Theme::Auto);   // explicit choice
```

Theme resolution: the underlying signal is `Signal<ColorMode>` from `leptos_use::use_color_mode` and is not exposed as a field. Read with `app.get_theme()` (reactive) or `app.get_theme_untracked()` (event handlers); both resolve `Auto` against the system media query and return a `Theme`. Mutate with `app.set_theme(theme)` or the `app.toggle_light_dark()` shortcut.

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

Components like `ContextMenu` require a `(ReadSignal<bool>, WriteSignal<bool>)` pair, but the source of truth is a `Split<Option<T>>` on `WorkbookState`. Two `Effect`s sync in both directions: one writes the local pair when the state signal changes, the other clears state when the local signal closes. The cycle terminates because each Effect writes only to things it doesn't read, and Leptos skips no-op writes. See `building-components.md` (canvas-sourced context menus) for the full pattern.

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
| `named_ranges_modal_open` | `Split<bool>` | Whether the Named Ranges manager dialog is mounted. Drives the `<Show>` wrapper around `<Modal>` in `components/named_ranges/mod.rs`. See [modal.md](modal.md). |
| `events` | `EventBus` | Per-category event signals. |

### AppState fields

| Field | Type | Purpose |
|-------|------|---------|
| _(theme)_ | `Signal<ColorMode>` (private, from `leptos_use::use_color_mode`) | User's theme preference. Public surface: `get_theme()` / `get_theme_untracked()` / `set_theme(Theme)` / `toggle_light_dark()`. |
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

See `adding-actions.md` for the action pipeline that produces these events.
