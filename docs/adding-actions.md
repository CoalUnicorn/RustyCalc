# Adding Keyboard Actions

Every user-triggered keyboard action follows the same three-step process.
The system splits key handling into classification (pure) and execution (side-effects).

## Architecture

```
KeyboardEvent
  -> workbook.rs on_keydown (guards, point-mode pre-check)
  -> classify_key()          pure: key string -> SpreadsheetAction
  -> execute()               side-effects: model mutation, signals, persistence
```

Two exceptions bypass `execute()`:
- **Clipboard** (`Copy`/`Cut`/`Paste`) — needs the `AppClipboard` store and async OS clipboard APIs. Handled inline in `workbook.rs`.
- **Point-mode arrows** — needs the textarea cursor position from the DOM. Runs as a pre-check in `workbook.rs` before `classify_key` is called.

## Adding a new action

### 1. Add a variant to `SpreadsheetAction`

Name it after the user's *intent*, not the key.

```rust
// Good:
DuplicateRow,
// Bad:
CtrlD,
```

### 2. Add a branch to `classify_key`

Pick the right modifier block and map the key string to the new variant.

```rust
// inside the `ctrl && !shift && !alt` block:
"d" => return Some(DuplicateRow),
```

`classify_key` is a pure function — no DOM access, no signal writes, no model mutations.

### 3. Add an arm to `execute`

Use the `mutate()` helper for model mutations. Pass `Eval::Yes` when formula
results may change (cell writes, row/column inserts/deletes). Pass `Eval::No`
for navigation or selection changes.

```rust
SpreadsheetAction::DuplicateRow => {
    mutate(model, state, Eval::Yes, |m| {
        let v = m.get_selected_view();
        m.insert_rows(v.sheet, v.row + 1, 1).ok();
        // copy row data...
    });
}
```

## Modifying an existing action

- To change the key binding: edit only `classify_key`.
- To change what it does: edit only `execute`.
- To change the name: rename the variant and update both.
