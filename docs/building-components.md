# Building Components in RustyCalc

Practical guide for writing, debugging, and structuring Leptos components in this codebase. Covers patterns we've settled on after hitting real compiler errors and rendering bugs.

See also: [leptos-patterns.md](leptos-patterns.md) for the reactive model and signal conventions.

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

### 4. Add styles to `style.css`

All static styles go in `style.css` (Trunk hashes and minifies it). Use `class=` in the view, not inline `style=`. Only use inline `style=` for values computed at runtime (pixel positions, per-instance colors).

```css
.toolbar { display: flex; align-items: center; height: 36px; }
.toolbar-btn { padding: 0 10px; font-size: 12px; cursor: pointer; }
```

### 5. Check it compiles

```
cargo check --target wasm32-unknown-unknown
```

Run this often. Leptos macro errors can be cryptic — catching them early in small increments saves time.

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
    let btn_class = move || if is_frozen() { "toolbar-btn active" } else { "toolbar-btn" };

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
SheetTabBar          (pub)   — layout, add button, <For> loop
├── SheetTab         (priv)  — one tab: click, dblclick, chevron menu
├── RenameInput      (priv)  — inline rename: keydown, blur, commit
├── TabContextMenu   (priv)  — right-click menu: rename, color, hide, delete
└── AllSheetsMenu    (priv)  — hamburger dropdown: navigate, unhide
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

## Reactive closures: the redraw subscription

The IronCalc model sits outside Leptos's signal graph. To read model state reactively, subscribe to `state.redraw`:

```rust
let is_frozen = move || {
    let _ = state.redraw.get();   // subscribe to changes
    model.with_value(|m| m.frozen_panes().is_frozen())
};
```

Forgetting `state.redraw.get()` is the #1 bug: the closure runs once at mount and never updates. If a value is stale after clicking or typing, check this first.

## Mutating the model

All model mutations follow the same shape:

```rust
model.update_value(|m| {
    warn_if_err(m.set_frozen_rows_count(sheet, 0), "set_frozen_rows_count");
});
state.request_redraw();
```

1. `model.update_value(|m| { ... })` — mutable borrow of UserModel
2. `warn_if_err(result, "context")` — log failures to browser console instead of silently swallowing with `.ok()`
3. `state.request_redraw()` — increment the redraw counter so closures and the canvas re-evaluate

For mutations that change formula results, also call `m.evaluate()` inside the closure.

After edits that should return keyboard focus to the grid:

```rust
crate::util::refocus_workbook();
```

## Popups, menus, and z-index

### The overflow trap

`.tab-bar-scroll` has `overflow-x: auto` for horizontal scrolling. Any `position: absolute` child inside it will be clipped — it won't appear above the canvas or other components.

Fix: use `position: fixed` and compute coordinates from the click event:

```rust
let on_chevron = move |ev: web_sys::MouseEvent| {
    menu_pos.set((ev.client_x(), ev.client_y()));
    menu_open.set(Some(sheet_idx));
};
```

```css
.tab-context-menu {
    position: fixed;
    z-index: 1100;
}
```

```rust
<div
    class="tab-context-menu"
    style=move || {
        let (x, y) = menu_pos.get();
        format!("left:{x}px;bottom:calc(100vh - {y}px + 4px);")
    }
>
```

### Click-away dismiss

Add an invisible full-screen backdrop behind the menu:

```rust
<div class="click-away-backdrop" on:click=move |_| menu_open.set(None) />
<div class="tab-context-menu"> ... </div>
```

```css
.click-away-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1099;   /* one below the menu */
}
```

The backdrop catches clicks outside the menu and closes it. The menu itself has `z-index: 1100` so it paints on top.

## Common compiler errors and fixes

### `FnOnce` vs `Fn` inside `<Show>`

```
error: expected a `Fn()` closure, found `FnOnce()`
```

`<Show>` children must be `Fn` (called each time the condition becomes true). A closure that moves a non-`Copy` value becomes `FnOnce`.

Fix: don't pass closures as props into sub-components rendered inside `<Show>`. Instead, have the sub-component call `expect_context()` and derive values from the model directly. All context types (`WorkbookState`, `ModelStore`) are `Copy`.

```rust
// Bad — closure prop captured by move, becomes FnOnce
<Show when=move || selected()>
    <TabActions on_delete=on_delete />   // on_delete is FnOnce
</Show>

// Good — sub-component pulls context, everything is Copy
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
// Bad — `left` dropped at end of else-if arm
let color = if let Some(ref bl) = style.border.left {
    bl.color.as_deref().unwrap_or("grey")
} else {
    let left = model.get_cell_style(sheet, row, col - 1);  // temporary
    left.fill.fg_color.as_deref().unwrap_or("grey")        // dangling ref
};

// Good — hoist the temporary
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
    // both closures work — `color` is Copy
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

- **Console warnings**: `warn_if_err` logs IronCalc errors as `[ironcalc] context: message`. If a mutation silently fails, check the console.
- **Reactive not updating?** Add `web_sys::console::log_1(&"closure ran".into())` inside the closure. If it doesn't print after a mutation, you forgot `state.redraw.get()`.
- **Element inspector**: Leptos CSR renders real DOM nodes. Inspect elements normally — no virtual DOM indirection.
- **Canvas debugging**: The grid is a `<canvas>`, not DOM elements. You can't inspect individual cells. Add `web_sys::console::log_1(...)` in `renderer.rs` to trace draw calls, but remove them before committing — they fire thousands of times per frame.

### WASM panics

WASM panics show as `unreachable` in the browser console with a cryptic stack trace. The stack points to wasm function indices, not Rust line numbers. To get readable panics:

1. Build in dev mode (the default — `trunk serve` without `--release`)
2. Look for the `panicked at src/file.rs:line` message in the console output — it's usually there but buried in the stack

### Signal debugging

If a component renders stale data:

1. Verify the closure subscribes to `state.redraw.get()`
2. Verify the mutation calls `state.request_redraw()` after `model.update_value`
3. Check if you're reading with `get_untracked()` when you meant `get()` — untracked reads don't subscribe

## File checklist for a new component

- [ ] `src/components/my_component.rs` — the component code
- [ ] `src/components/mod.rs` — add `pub mod my_component;`
- [ ] `style.css` — add CSS classes
- [ ] Parent component's `view!` — add `<MyComponent />`
- [ ] `cargo check --target wasm32-unknown-unknown` — compiles
- [ ] `wasm-pack test --headless --firefox` — tests still pass
