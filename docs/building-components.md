# Building Components

Patterns for writing and debugging Leptos components in RustyCalc.
See also: [leptos-patterns.md](leptos-patterns.md) for reactivity and signal conventions.

## Starting a new component

### 1. Create the file and register it

```
src/components/toolbar.rs    <- new file
src/components/mod.rs        <- add: pub mod toolbar;
```

### 2. Minimal skeleton

```rust
use leptos::prelude::*;

use crate::state::{ModelStore, WorkbookState};

#[component]
pub fn Toolbar() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    view! {
        <div class="toolbar">
            "placeholder"
        </div>
    }
}
```

### 3. Wire it into the layout

In `workbook.rs`, add `<Toolbar />` where it belongs in the view:

```rust
use crate::components::toolbar::Toolbar;

view! {
    <div id="workbook" class="workbook" tabindex="0" on:keydown=on_keydown>
        <FileBar />
        <Toolbar />       // <- new
        <FormulaBar />
        <Worksheet />
        <SheetTabBar />
    </div>
}
```

### 4. Create css file for component

Each UI component gets its own CSS file in `styles/` with a short 2-3 char prefix.
All static styles go there (Trunk hashes and minifies). Use `class=` in the view, not
inline `style=`. Only use inline `style=` for values computed at runtime (pixel
positions, per-instance colors).

```css
/* styles/toolbar.css  — prefix: tb- */
.tb { display: flex; align-items: center; height: 36px; }
.tb .tb-btn { padding: 0 10px; font-size: 12px; cursor: pointer; }
```

Then add `@import "toolbar.css";` to `styles/index.css`. See `styles/README.md`
for the full prefix table and naming conventions.

Always include theme variable declarations so the component reacts to light/dark
switching. Omitting them leaves the element white regardless of the active theme:

```css
.tb {
    background: var(--bg-secondary);
    color: var(--text-primary);
    border-bottom: 1px solid var(--border-color);
}
```

Available variables: `--bg-primary`, `--bg-secondary`, `--border-color`,
`--border-inner`, `--text-primary`, `--text-dim`, `--text-strong`, `--accent`,
`--btn-bg`. All defined in `index.html` on `:root` and `[data-theme="dark"]`.

### 5. Check it compiles

```
cargo check --target wasm32-unknown-unknown
```

Run this often. Leptos macro errors can be cryptic, so catching them early in small increments saves time.

## Component structure patterns

### Small component: single function

For something like a button with a click handler and reactive class, keep it in one `#[component]` function. The freeze pane button is a good example:

```rust
#[component]
fn FreezePane() -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let is_frozen = move || { /* reactive query */ };
    let on_click = move |_: web_sys::MouseEvent| { /* model mutation */ };
    let btn_class = move || if is_frozen() { "tb-btn active" } else { "tb-btn" };

    view! {
        <button class=btn_class on:click=on_click>"❄ Freeze"</button>
    }
}
```

Guideline: if the view block fits on screen and there's one set of event handlers, a single function is fine.

### Medium component: parent + private children

When a component has distinct interactive sections, split them into private sub-components in the same file. The parent coordinates shared signals; children pull their own context.

`sheet_tab_bar.rs` follows this pattern:

```
SheetTabBar          (pub)   (layout, add button, <For> loop)
├── SheetTab         (priv)  (one tab: click, dblclick, chevron menu)
│   └── uses InlineRenameInput from inline_rename.rs
└── AllSheetsMenu    (priv)  (hamburger dropdown: navigate, unhide)
```

Shared state between siblings (e.g. "which tab's menu is open") is passed as `RwSignal` props from the parent:

```rust
// Parent creates the signal
let menu_open: RwSignal<Option<u32>> = RwSignal::new(None);

// Children receive it as a prop
#[component]
fn SheetTab(sheet_idx: u32, menu_open: RwSignal<Option<u32>>) -> impl IntoView { ... }
```

`RwSignal` is `Copy`, so it works in closures inside `<Show>` blocks (which require `Fn`, not `FnOnce`).

### When to split vs. inline

Split into a sub-component when:
- The section has its own event handlers (keydown, blur, click)
- The section has local signals (open/closed, hover state)
- The view block would push the parent past ~80 lines

Keep inline when:
- It's a static label or simple conditional text
- No event handlers or local state

## Reactive closures: subscribing to the event bus

The IronCalc model sits outside Leptos's signal graph. To read model state reactively, subscribe to the relevant `state.events` category signal:

```rust
let is_frozen = move || {
    let _ = state.events.navigation.get(); // subscribe - re-runs on any navigation event
    model.with_value(|m| m.frozen_panes().is_frozen())
};
```

Pick the category that matches what can change the value you're reading:

| Value comes from | Subscribe to |
|-----------------|-------------|
| Cell values, formulas | `state.events.content.get()` |
| Column widths, row heights, fonts | `state.events.format.get()` |
| Sheet list, row/col counts | `state.events.structure.get()` |
| Active cell, selected sheet | `state.events.navigation.get()` |
| Light/dark theme | `state.events.theme.get()` |

Forgetting to subscribe is the #1 staleness bug: the closure runs once at mount and never updates. If a value is stale after clicking or typing, check this first.

Subscribing to more categories than necessary causes extra re-renders. A component that only cares about sheet switches should subscribe to `structure`, not `content`.

## Mutating the model

All model mutations go through `mutate` / `try_mutate` in `src/model/frontend_model.rs`. These wrap `pause_evaluation` / `resume_evaluation` so formulas never evaluate twice per keystroke.

```rust
use crate::model::{mutate, try_mutate, EvaluationMode};

// Infallible (navigation, UI state):
mutate(model, EvaluationMode::Deferred, |m| {
    m.set_frozen_rows_count(sheet, 0);
});

// Fallible (cell writes, structural changes):
if let Err(e) = try_mutate(model, EvaluationMode::Immediate, |m| {
    m.insert_columns(sheet, col, 1).map_err(TabError::Engine)
}) {
    state.status.set(Some(StatusMessage::Error(e.to_string())));
    return;
}
```

`EvaluationMode::Immediate` calls `evaluate()` once after the closure — use when the mutation affects formula results (cell edits, row/col insert/delete, paste). `EvaluationMode::Deferred` skips evaluation — use for navigation, formatting, and UI-only changes.

After the mutation, emit the right typed event (see `events.rs`):

```rust
state.emit_event(SpreadsheetEvent::Structure(
    StructureEvent::columns_inserted(Location::new(sheet, col, 1)),
));
```

After edits that should return keyboard focus to the grid:

```rust
crate::util::refocus_workbook();
```

## When to move logic out of a component

Closures inside components are fine for isolated UI interactions (toggle a signal, call one mutation). Move logic to `src/input/` when a closure does more than one type of thing to application state.

A useful test: if the closure would need to be copied to add a second UI entry point (keyboard shortcut, context menu item, toolbar button), it belongs in an action module.

### The line between component and action

| Stays in component | Moves to `src/input/` |
|-------------------|----------------------|
| `window.confirm()` dialogs | Multi-step state transitions |
| Reading event coordinates (`client_x`, `client_y`) | Model mutations + state reset + event emission |
| Local UI state (open/closed, hover) | Any logic you'd want a keyboard shortcut to trigger |
| DOM refs and focus management | Transitions that touch 3+ reactive signals |

### Recognising the pattern

A closure that: saves current state → loads new state → resets transient UI fields → emits an event is a workflow, not a UI handler. The `switch_workbook`, `create_workbook`, and `delete_workbook` operations in `left_drawer.rs` all had this shape before extraction:

```rust
// Bad: workflow buried in a UI closure
let switch_workbook = move |uuid: WorkbookId| {
    model.with_value(|m| storage::save(&cur_uuid, m));
    let new_model = storage::load(&uuid).unwrap();
    model.update_value(|m| *m = new_model);
    storage::set_selected_uuid(&uuid);
    state.current_uuid.set(Some(uuid));
    state.editing_cell.set(None);
    state.drag.set(DragState::Idle);
    state.emit_event(SpreadsheetEvent::Content(ContentEvent::GenericChange));
};
```

After extraction, the component holds only the UI concern:

```rust
// Good: component owns the decision; action module owns the transition
let switch_workbook = move |uuid: WorkbookId| {
    execute_workbook(&WorkbookAction::Switch(uuid), model, &state, app);
};
```

See `src/input/workbook.rs` for the full pattern and `docs/adding-actions.md` for how to add new variants.

## Popups, menus, and z-index

### The overflow trap

Any scrollable container with `overflow: auto` will clip `position: absolute` children - they won't appear above other components. Always use `position: fixed` for menus and popups, with coordinates read from `ev.client_x()` / `ev.client_y()`.

`ContextMenu` handles this automatically (see below).

### Adding a context menu

Use `ContextMenu` + `ContextMenuItem` from `src/components/context_menu.rs`. The component owns the backdrop and fixed positioning; the caller owns the open/pos signals.

```rust
use crate::components::context_menu::{ContextMenu, ContextMenuItem, ContextMenuSeparator};
```

**Signal setup** (in the parent component or sub-component that owns the menu):

```rust
let (menu_open, set_menu_open) = signal(false);
let (menu_pos,  set_menu_pos)  = signal((0i32, 0i32));
```

**Trigger** - wire `on:contextmenu` on the element that should open it:

```rust
let on_right_click = move |ev: web_sys::MouseEvent| {
    ev.prevent_default();                              // suppress browser menu
    set_menu_pos.set((ev.client_x(), ev.client_y()));
    set_menu_open.set(true);
};
```

**View**:

```rust
view! {
    <div class="col-header" on:contextmenu=on_right_click>
        {col_label}
    </div>

    <ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
        <ContextMenuItem on_click=move || { /* insert col */ }>
            "Insert column"
        </ContextMenuItem>
        <ContextMenuItem on_click=move || { /* delete col */ } destructive=true>
            "Delete column"
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem on_click=move || { /* hide col */ }>
            "Hide"
        </ContextMenuItem>
    </ContextMenu>
}
```

`ContextMenuItem` automatically closes the menu after its `on_click` fires - no manual `set_menu_open.set(false)` needed in each handler.

#### `above_anchor`

For menus attached to a bottom bar (e.g. sheet tabs), pass `above_anchor=true`. The menu renders above the click point instead of below:

```rust
<ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos above_anchor=true>
    ...
</ContextMenu>
```

#### Row / column header example

Headers sit inside the canvas area where there are no scrollable wrappers, so a standard right-click menu works without any extra considerations. A full header context menu sub-component looks like:

```rust
use crate::model::{try_mutate, EvaluationMode};

#[component]
fn ColHeaderMenu(col: i32) -> impl IntoView {
    let state = expect_context::<WorkbookState>();
    let model = expect_context::<ModelStore>();

    let (open, set_open) = signal(false);
    let (pos, set_pos)   = signal((0i32, 0i32));

    let on_contextmenu = move |ev: web_sys::MouseEvent| {
        ev.prevent_default();
        set_pos.set((ev.client_x(), ev.client_y()));
        set_open.set(true);
    };

    let on_insert = move || {
        let sheet = model.with_value(|m| m.get_selected_view().sheet);
        if let Err(e) = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.insert_columns(sheet, col, 1).map_err(|e| e.to_string())
        }) {
            state.status.set(Some(StatusMessage::Error(e)));
            return;
        }
        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::columns_inserted(Location::new(sheet, col, 1)),
        ));
        crate::util::refocus_workbook();
    };

    let on_delete = move || {
        let sheet = model.with_value(|m| m.get_selected_view().sheet);
        if let Err(e) = try_mutate(model, EvaluationMode::Immediate, |m| {
            m.delete_columns(sheet, col, 1).map_err(|e| e.to_string())
        }) {
            state.status.set(Some(StatusMessage::Error(e)));
            return;
        }
        state.emit_event(SpreadsheetEvent::Structure(
            StructureEvent::columns_deleted(Location::new(sheet, col, 1)),
        ));
        crate::util::refocus_workbook();
    };

    view! {
        <div class="col-header" on:contextmenu=on_contextmenu>
            {col_label(col)}
        </div>

        <ContextMenu open=open set_open=set_open pos=pos>
            <ContextMenuItem on_click=on_insert>"Insert column"</ContextMenuItem>
            <ContextMenuItem on_click=on_delete destructive=true>"Delete column"</ContextMenuItem>
        </ContextMenu>
    }
}
```

Key points:
- `on:contextmenu` captures `client_x`/`client_y` before storing position - these are only valid during the event.
- The `ContextMenu` is placed as a sibling of the header div, not a child. Children inside scrollable containers get clipped.
- Emit a `Structure` event after the mutation so subscribers (e.g. the canvas) re-render.
- Call `refocus_workbook()` to return keyboard focus to the grid after the menu closes.

#### Canvas-sourced context menus (overlay pattern)

When the interactive element is canvas-drawn (not a DOM node), there's nothing
to attach `on:contextmenu` to. Instead, use a two-component split:

1. **Mouse handler** detects the right-click, identifies the target, and writes
   to `state.context_menu: Split<Option<ContextMenuState>>`.
2. **Overlay component** lives at workbook layout level and reads
   `state.context_menu`. Two `Effect`s bridge the external signal to the
   `(ReadSignal<bool>, WriteSignal<bool>)` pair that `ContextMenu` requires:

```rust
let (menu_open, set_menu_open) = signal(false);
let (menu_pos,  set_menu_pos)  = signal((0i32, 0i32));

// When Some(ctx) arrives, update menu_pos and open the menu.
// When None arrives, close it.
Effect::new(move |_| {
    match state.context_menu.get() {
        Some(ctx) => {
            set_menu_pos.set((ctx.x, ctx.y));
            set_menu_open.set(true);
        }
        None => set_menu_open.set(false),
    }
});

// When menu_open transitions true -> false (outside click or item action),
// clear state.context_menu so nothing else thinks the menu is still open.
Effect::new(move |prev: Option<bool>| {
    let is_open = menu_open.get();
    if prev == Some(true) && !is_open {
        state.context_menu.set(None);
    }
    is_open   // becomes `prev` on the next run
});
```

The cycle terminates cleanly: when Effect 2 clears `state.context_menu`,
Effect 1 fires and sets `menu_open = false` — a no-op since it's already false.

Because `ContextMenuItem` closes the menu via `use_context::<WriteSignal<bool>>()`,
and that context doesn't propagate through a reactive `move || match` closure boundary,
actions dispatched from inside the reactive children block must clear `state.context_menu`
explicitly. Define a `dispatch` closure that does both:

```rust
let dispatch = move |action: StructAction| {
    state.context_menu.set(None);
    execute(&SpreadsheetAction::Structure(action), model, &state);
};
```

To switch content between column and row items, put a reactive closure inside
`ContextMenu`'s children. Because `ContextMenu` mounts its children once via
`FnOnce`, the outer wiring is static; the reactive closure re-runs on every
`state.context_menu` change:

```rust
<ContextMenu open=menu_open set_open=set_menu_open pos=menu_pos>
    {move || match state.context_menu.get() {
        Some(ctx) => match ctx.target {
            HeaderContextMenu::Column(col) => view! { /* col items */ }.into_any(),
            HeaderContextMenu::Row(row)    => view! { /* row items */ }.into_any(),
        },
        None => ().into_any(),
    }}
</ContextMenu>
```

See `src/components/header_context_menu.rs` for the full implementation and
`src/input/mouse.rs::handle_contextmenu` for the handler that writes to
`state.context_menu`.

#### `ContextMenuButton` (toggle trigger)

For button-triggered menus (not right-click), use `ContextMenuButton` instead of wiring `on:contextmenu` manually:

```rust
<ContextMenuButton set_open=set_open set_pos=set_pos class="header-btn">
    "⋮"
</ContextMenuButton>
<ContextMenu open=open set_open=set_open pos=pos>
    ...
</ContextMenu>
```

`ContextMenuButton` captures coordinates and toggles open state. It's a convenience wrapper - use it when the trigger is a visible button element.

### Adding an inline rename

Use `InlineRenameInput` from `src/components/inline_rename.rs`. The component
owns focus management, keyboard dispatch, and the double-commit guard. The
caller provides domain logic via `on_commit` / `on_cancel` callbacks.

```rust
use crate::components::inline_rename::InlineRenameInput;

// In the parent component:
let (renaming, set_renaming) = signal(false);

let on_commit = Callback::new(move |new_name: String| {
    if !new_name.trim().is_empty() {
        // domain: mutate model, save, emit event
    }
    set_renaming.set(false);
});

let on_cancel = Callback::new(move |()| {
    set_renaming.set(false);
});

view! {
    <Show when=move || renaming.get()>
        <InlineRenameInput
            value=current_name()
            on_commit=on_commit
            on_cancel=on_cancel
            class="my-rename-input"
        />
    </Show>
}
```

`on_cancel` is optional. When omitted, Escape calls `on_commit` with the
original value — useful when the commit callback already handles no-ops.

## Common compiler errors and fixes

### `FnOnce` vs `Fn` inside `<Show>`

```
error: expected a `Fn()` closure, found `FnOnce()`
```

`<Show>` children must be `Fn` (called each time the condition becomes true). A closure that moves a non-`Copy` value becomes `FnOnce`.

Fix: don't pass closures as props into sub-components rendered inside `<Show>`. Instead, have the sub-component call `expect_context()` and derive values from the model directly. All context types (`WorkbookState`, `ModelStore`) are `Copy`.

```rust
// Bad: closure prop captured by move, becomes FnOnce
<Show when=move || selected()>
    <TabActions on_delete=on_delete />   // on_delete is FnOnce
</Show>

// Good: sub-component pulls context, everything is Copy
<Show when=move || selected()>
    <TabActions sheet_idx=sheet_idx />
</Show>
```

### `>` parsed as HTML tag close

```
error: expected closing tag
```

The `view!` macro parses `>` as an HTML tag boundary. Wrap comparisons in braces:

```rust
// Wrong:
<Show when=move || count() > 1>

// Right:
<Show when=move || { count() > 1 }>
```

### Borrow doesn't live long enough in reactive closures

```
error[E0597]: borrowed value does not live long enough
```

A temporary created inside a closure branch gets dropped before the return value can use it. Fix: hoist the temporary before the branch so its lifetime spans the full expression.

```rust
// Bad: `left` dropped at end of else-if arm
let color = if let Some(ref bl) = style.border.left {
    bl.color.as_deref().unwrap_or("grey")
} else {
    let left = model.get_cell_style(sheet, row, col - 1);  // temporary
    left.fill.fg_color.as_deref().unwrap_or("grey")        // dangling ref
};

// Good: hoist the temporary
let left_nb = if style.border.left.is_none() && col > 1 {
    Some(model.get_cell_style(sheet, row, col - 1))
} else {
    None
};
let color = if let Some(ref bl) = style.border.left {
    bl.color.as_deref().unwrap_or("grey")
} else if let Some(ref left) = left_nb {
    left.fill.fg_color.as_deref().unwrap_or("grey")
} else {
    "grey"
};
```

### `impl Fn` prop used twice

```
error[E0382]: use of moved value
```

An `impl Fn()` prop is not `Copy`. If two closures need to read it, wrap it in a signal:

```rust
#[component]
fn TabColorSwatch(
    tab_color: impl Fn() -> Option<String> + Send + Sync + 'static,
) -> impl IntoView {
    let color = Signal::derive(tab_color);  // now Copy

    let dot_class = move || if color.get().is_some() { "has-color" } else { "no-color" };
    let dot_bg = move || color.get().map(|c| format!("background:{c};")).unwrap_or_default();
    // both closures work since `color` is Copy
}
```

## Debugging

### Build commands

```sh
# Type check (fast, catches most errors)
cargo check --target wasm32-unknown-unknown

# Dev server with hot reload
trunk serve

# Tests (requires a real browser for DOM APIs)
wasm-pack test --headless --firefox

# Tauri desktop shell
cargo tauri dev
```

### Browser DevTools

- **Status bar errors**: failed mutations set `state.status` to `StatusMessage::Error(msg)`, displayed in the status bar. If a mutation silently fails, check both the status bar and whether your error is being swallowed before reaching `state.status.set(...)`.
- **Reactive not updating?** Add `web_sys::console::log_1(&"closure ran".into())` inside the closure. If it doesn't print after a mutation, you forgot to subscribe to the right `state.events` signal (e.g. `let _ = state.events.content.get();`).
- **Element inspector**: Leptos CSR renders real DOM nodes. Inspect elements normally (no virtual DOM indirection).
- **Canvas debugging**: The grid is a `<canvas>`, not DOM elements. You can't inspect individual cells. Add `web_sys::console::log_1(...)` in `renderer.rs` to trace draw calls, but remove them before committing since they fire thousands of times per frame.

### WASM panics

WASM panics show as `unreachable` in the browser console with a cryptic stack trace. The stack points to wasm function indices, not Rust line numbers. To get readable panics:

1. Build in dev mode (the default: `trunk serve` without `--release`)
2. Look for the `panicked at src/file.rs:line` message in the console output (it's usually there but buried in the stack)

### Signal debugging

If a component renders stale data:

1. Verify the closure subscribes to the right `state.events` signal (e.g. `let _ = state.events.content.get();`). The category must match what the mutation emits.
2. Verify the mutation calls `state.emit_event(...)` after `mutate`/`try_mutate`.
3. Check if you're reading with `get_untracked()` when you meant `get()` (untracked reads don't subscribe).

## File checklist for a new component

- [ ] `src/components/my_component.rs` (the component code)
- [ ] `src/components/mod.rs` (add `pub mod my_component;`)
- [ ] `styles/my_component.css` (CSS with a new prefix)
- [ ] `styles/index.css` (add `@import "my_component.css";`)
- [ ] Parent component's `view!` (add `<MyComponent />`)
- [ ] `cargo check --target wasm32-unknown-unknown` (compiles)
- [ ] `wasm-pack test --headless --firefox` (tests still pass)
