# Leptos Patterns in RustyCalc

This documents the patterns and conventions used in our Leptos CSR codebase.
Read this before writing new components.

## Component lifecycle

A `#[component]` function runs **once** at mount time. Leptos then re-runs only
the individual closures whose signals changed. This is different from React,
which re-runs the entire component function on every state change.

```rust
#[component]
pub fn MyComponent() -> impl IntoView {
    // This line runs once.
    let state = expect_context::<WorkbookState>();

    // This closure is registered as a subscription.
    // Leptos calls it whenever a content event is emitted.
    let value = move || {
        let _ = state.events.content.get(); // subscribe to content changes
        model.with_value(|m| m.active_cell_content())
    };

    view! { <span>{value}</span> }
    //             ^^^^^^^ only this text node updates
}
```

## Shared state: context, not props

We use `provide_context` / `expect_context` instead of prop drilling.
All shared state lives in `WorkbookState` (UI signals) and `ModelStore`
(the ironcalc `UserModel`).

```rust
// In App (once):
provide_context(wb_state);
provide_context(model);

// In any descendant component:
let state = expect_context::<WorkbookState>();
let model = expect_context::<ModelStore>();
```

`WorkbookState` is `Copy` since all its fields are `Split<T>` (which wraps a
`ReadSignal<T>`/`WriteSignal<T>` pair - both arena indices), `NodeRef<T>`, or
similarly arena-allocated types. Closures capture them by implicit copy
with zero allocation. **Never clone WorkbookState or create aliases like
`let state_md = state.clone()`.**

## Reactive closures and the event bus

The IronCalc model lives outside Leptos's reactive system. To bridge the gap,
`WorkbookState` has a typed `EventBus` (`state.events`) with six category signals.
After every model mutation an event is emitted, and any closure that subscribed
to that category re-runs.

```rust
let cell_address = move || {
    let _ = state.events.navigation.get(); // subscribe to navigation events
    model.with_value(|m| {
        let ac = m.active_cell();
        format!("{}{}", col_name(ac.column), ac.row)
    })
};
```

Without a `state.events.*` subscription, the closure would compute once at mount
and never update.

Subscribe to the most specific category that matches what can change the value:

| What changes | Subscribe to |
|---|---|
| Cell values, formulas | `state.events.content.get()` |
| Fonts, colors, column widths | `state.events.format.get()` |
| Sheet list, inserted/deleted rows or cols | `state.events.structure.get()` |
| Active cell, selected sheet, scroll | `state.events.navigation.get()` |
| Drag mode, context menu, point mode | `state.events.mode.get()` |
| Light/dark theme | `state.events.theme.get()` |

Over-subscribing causes unnecessary re-renders - a component that only cares
about sheet switches should subscribe to `structure`, not `content`.

## View bindings

```rust
view! {
    // Text content (re-evaluates when the closure's signals change)
    <div>{cell_address}</div>

    // DOM property binding (use for <input>/<textarea> .value)
    <input prop:value=display_text />

    // HTML attribute binding (sets the attribute, not the JS property)
    <input value="initial" />

    // Event handler
    <input on:input=on_input on:keydown=on_keydown />

    // DOM element reference
    <input node_ref=input_ref />

    // Reactive style (closure returns a string)
    <div style=move || if active() { "color:blue" } else { "color:gray" } />

    // Conditional rendering
    <Show when=move || editing()>
        <Editor />
    </Show>
}
```

`prop:value` vs `value`: use `prop:value` for inputs so the displayed text
stays in sync with your signal after the user types. Plain `value` only sets
the initial HTML attribute.

## Comparisons inside `view!`

The `>` character closes HTML tags inside `view!`. Wrap Rust comparisons in
braces so the macro doesn't misparse them:

```rust
// Wrong (`>` parsed as tag close):
<Show when=move || count() > 1>

// Correct:
<Show when=move || { count() > 1 }>
```

## Event bubbling for commit/cancel

Cell editor and formula bar intercept `Enter`/`Tab`/`Escape` with
`prevent_default()` to suppress browser defaults (newline, tab-focus,
etc.), but they do **not** call `stop_propagation()`. The event bubbles
up to `Workbook`'s `on:keydown`, which calls `classify_key` -> `execute`
to commit or cancel the edit.

The `Workbook` keydown guard skips `<input>` and `<textarea>` events
when not editing, so keystrokes in panel forms (Named Ranges, etc.)
don't trigger spreadsheet actions.

## `<For>` lists

Use `<For>` for dynamic lists. The `key` must be a stable identity so
Leptos can diff additions/removals without recreating everything.

```rust
<For
    each=move || visible_sheets()
    key=|(sheet_id, sheet_idx)| (*sheet_id, *sheet_idx)
    children=move |(_, sheet_idx)| {
        view! { <SheetTab sheet_idx=sheet_idx /> }
    }
/>
```

Derive display state (name, color, is_selected) reactively **inside** the
child component rather than capturing it from the `each` data. Captured
values go stale when the model changes; reactive closures that subscribe
to the right `state.events` category stay current.

## Sub-components and `<Show>`

Extract focused sub-components when a section has its own state or event
handlers. Use `<Show>` for conditional rendering instead of
`if ... { view!{}.into_any() } else { view!{}.into_any() }`.

```rust
<Show when=move || is_renaming() fallback=move || view! { <span>{name}</span> }>
    <RenameInput sheet_idx=sheet_idx />
</Show>
```

`<Show>` children must be `Fn` (callable multiple times as the section
mounts/unmounts when the condition toggles). Closures that capture non-`Copy`
values become `FnOnce` and won't compile inside `<Show>`. Fix: derive
values from context inside the child component (all context types are `Copy`).

## leptos-use hooks

We use `leptos-use` (v0.15, compatible with Leptos 0.7) to replace manual
`web_sys` boilerplate. These hooks handle cleanup automatically on unmount.

| Instead of | Use |
|---|---|
| Manual `ResizeObserver` + `Closure::new` + `forget()` | `use_resize_observer(node_ref, callback)` |
| Manual `setInterval` + `Closure::wrap` + `forget()` | `use_interval_fn(callback, ms)` |

Avoid `Closure::new` + `.forget()` for browser API subscriptions since it leaks
memory and never cleans up. If `leptos-use` has a hook for it, use the hook.
