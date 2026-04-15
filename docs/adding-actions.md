# Adding Actions

Every user action follows the same pipeline. Keyboard shortcuts and toolbar
buttons both produce a `SpreadsheetAction` which is dispatched to the
appropriate category handler.

## Architecture

```
KeyboardEvent / Toolbar click
  ->  classify_key()  or  SpreadsheetAction::toggle_bold()
  ->  SpreadsheetAction (wrapper enum)
  ->  execute()
      - Nav(a)       -> execute_nav()       nav.rs
      - Edit(a)      -> execute_edit()      edit.rs
      - Format(a)    -> execute_format()    format.rs
      - Structure(a) -> execute_struct()    structure.rs
      - Copy/Cut/Paste -> handled inline in workbook.rs

Direct-call actions (bypass execute()):
  ->  execute_workbook()   workbook.rs   (model replacement lifecycle)
  ->  execute_sheet()      sheet.rs      (sheet-level ops with storage save)
```

### File layout

```
src/input/
├── keyboard.rs        SpreadsheetAction, classify_key(), execute(), convenience constructors
├── error.rs         FormatError, StructError, NavError, EditError  (thiserror-derived)
├── nav.rs           NavAction      (arrows, page, home/end, sheet switch, select all)
├── edit.rs          EditAction     (start, commit, cancel)
├── format.rs        FormatAction   (bold, italic, underline, strikethrough, font size/family)
├── structure.rs     StructAction   (delete, clear, undo/redo, insert/delete rows/columns)
├── workbook.rs      WorkbookAction (switch, create, delete) — see below
├── sheet.rs         SheetAction    (select, add, delete, hide, unhide, rename, set color) — see below
├── mouse.rs         Worksheet mouse/wheel handlers (mousedown dispatch, resize drag, autofill,
│                    point-mode click, selection drag, scroll) — called from thin closures in
│                    worksheet.rs; bypasses execute() like clipboard and point-mode arrows
├── formula_input.rs (point-mode reference handling, separate from the action pipeline)
└── xlsx_io.rs       File import/export

src/model/
└── frontend_model.rs  mutate(), try_mutate(), EvaluationMode  (pause/resume wrappers)
```

### Three things bypass `execute()`

- **Clipboard** (`Copy`/`Cut`/`Paste`) needs the `AppClipboard` store and async OS clipboard APIs. Handled inline in `workbook.rs`.
- **Point-mode arrows** need the textarea cursor position from the DOM. Runs as a pre-check in `workbook.rs` before `classify_key` is called.
- **Sheet operations** (`SheetAction`) in `sheet.rs` need `storage::save()` after mutations and coordinate UI state (`add_recent_color`). Called directly from `SheetTabBar` and `AllSheetsMenu`.

## Adding a new action

### 1. Pick the right category

| Category    | File           | When to use                                     |
|-------------|----------------|--------------------------------------------------|
| `NavAction` | `nav.rs`       | Moving the cursor, switching sheets, selecting   |
| `EditAction`| `edit.rs`      | Starting, committing, or cancelling a cell edit  |
| `FormatAction` | `format.rs` | Changing visual style (font, bold, color, etc.)  |
| `StructAction` | `structure.rs` | Changing sheet structure (insert/delete rows, undo/redo) |

### 2. Add a variant to the sub-enum

Name it after the user's *intent*, not the key.

```rust
// In format.rs:
pub enum FormatAction {
    // ... existing variants ...
    /// Set horizontal alignment on the selected range.
    SetAlignment(HorizontalAlignment),
}
```

### 3. Add the handler in the same file

Each `execute_*` function returns `Result<(), XxxError>`. Use `try_mutate()` for
fallible model mutations — it handles pause/resume evaluation and surfaces the
error as the function's `Result`. Use plain `mutate()` for infallible arms.

```rust
use crate::input::error::FormatError;
use crate::model::{try_mutate, EvaluationMode};
```

See [performance-evaluation.md](performance-evaluation.md) for details on avoiding double evaluation.

Pass `EvaluationMode::Immediate` when formula results may change (cell writes,
row/column inserts/deletes). Pass `EvaluationMode::Deferred` for formatting changes
that don't affect formula output.

```rust
// In format.rs execute_format():
FormatAction::SetAlignment(align) => {
    let val = match align {
        HorizontalAlignment::Left => "left",
        HorizontalAlignment::Center => "center",
        HorizontalAlignment::Right => "right",
        HorizontalAlignment::General => "",
    };
    try_mutate(model, EvaluationMode::Deferred, |m| -> Result<(), FormatError> {
        let area = selection_area(m);
        m.update_range_style(&area, "alignment.horizontal", val)
            .map_err(FormatError::Engine)?;
        Ok(())
    })?;
}
```

Errors propagate up to `execute()` in `keyboard.rs`, which maps them all to
`String` and sets `state.status` to `StatusMessage::Error(msg)` — displayed in
the status bar. On success, `execute()` clears the status (`state.status.set(None)`).
Callers of `execute()` never see individual error types.

### 4. (Optional) Add a keyboard shortcut

If the action has a shortcut, add a branch to `classify_key()` in `keyboard.rs`.
Wrap the sub-action in the wrapper enum:

```rust
// inside the `ctrl && !shift && !alt` block in classify_key():
"l" => return Some(Format(FormatAction::SetAlignment(HorizontalAlignment::Left))),
"e" => return Some(Format(FormatAction::SetAlignment(HorizontalAlignment::Center))),
"r" => return Some(Format(FormatAction::SetAlignment(HorizontalAlignment::Right))),
```

`classify_key` is pure: no DOM access, no signal writes, no model mutations.

### 5. (Optional) Add a convenience constructor

If toolbar or other components will use this action, add a constructor on
`SpreadsheetAction` to avoid the deep nesting:

```rust
// In action.rs impl SpreadsheetAction:
pub fn set_alignment(align: HorizontalAlignment) -> Self {
    Self::Format(FormatAction::SetAlignment(align))
}
```

Then in the toolbar:
```rust
execute(&SpreadsheetAction::set_alignment(HorizontalAlignment::Center), model, &state);
```

### 6. No changes needed in workbook.rs

The workbook match uses `SpreadsheetAction::Nav(_) | Edit(_) | Format(_) | Structure(_)`,
so adding variants to a sub-enum doesn't require updating the match.

## Modifying an existing action

- To change the key binding: edit only `classify_key()` in `keyboard.rs`.
- To change what it does: edit only the `execute_*()` function in the category file.
- To rename a variant: update the sub-enum and its execute arm. If it has a
  keyboard shortcut, update `classify_key()`. If it has a convenience
  constructor, update that too.

## Testing

Tests live in `keyboard.rs` under `#[cfg(test)]`. Two kinds:

- **`classify_key` tests**: pure input/output, no browser needed (but run in browser anyway since the crate is wasm-only).
- **`execute` tests**: need a real browser environment (`Owner::new()`, `StoredValue::new_local(UserModel)`, `WorkbookState::new()`).

```rust
#[wasm_bindgen_test]
fn my_new_shortcut_works() {
    let mods = KeyMod { ctrl: true, shift: false, alt: false };
    assert_eq!(
        classify_key("l", mods, None),
        Some(SpreadsheetAction::Format(FormatAction::SetAlignment(HorizontalAlignment::Left)))
    );
}
```

Run with:
```
wasm-pack test --headless --firefox
```

---

## Workbook-level actions (`WorkbookAction`)

`SpreadsheetAction` covers mutations *within* the loaded model — cell edits, formatting, navigation. `WorkbookAction` covers operations that *replace* the model: switching to a different workbook, creating a new one, or deleting one. These live in `src/input/workbook.rs` and are called directly from components, not routed through `classify_key`/`execute`.

### When to use `WorkbookAction` vs `SpreadsheetAction`

Use `WorkbookAction` when the operation:
- Replaces the content of `ModelStore` (`model.update_value(|m| *m = ...`)
- Resets transient UI state (`editing_cell`, `drag`) because a new workbook starts fresh
- Touches `AppState.registry_version` (any change to which workbooks exist)

Use `SpreadsheetAction` for everything else — mutations to the currently loaded workbook.

### Calling it from a component

```rust
use crate::input::workbook::{execute_workbook, WorkbookAction};

// Switch to another workbook (saves current first)
execute_workbook(&WorkbookAction::Switch(uuid), model, &state, app);

// Create and activate a blank workbook
execute_workbook(&WorkbookAction::Create, model, &state, app);

// Delete a workbook (caller must confirm first — see below)
execute_workbook(&WorkbookAction::Delete(uuid), model, &state, app);
```

`execute_workbook` takes the same `model`, `&state`, `app` that components already hold from `expect_context`.

### Confirmation dialogs stay in the component

`WorkbookAction::Delete` assumes the user already confirmed. The `window.confirm()` call stays in the UI component because it's a presentation concern — the action module doesn't decide whether to prompt.

```rust
let delete_workbook = move |uuid: WorkbookId| {
    let wb_name = storage::load_registry()
        .get(&uuid)
        .map(|m| m.name.clone())
        .unwrap_or_default();
    let confirmed = web_sys::window()
        .and_then(|w| w.confirm_with_message(
            &format!("Delete '{wb_name}'? This cannot be undone.")
        ).ok())
        .unwrap_or(false);
    if confirmed {
        execute_workbook(&WorkbookAction::Delete(uuid), model, &state, app);
    }
};
```

### The `activate` helper

All three operations end with the same sequence: load model → set selected UUID in localStorage → update `current_uuid` → reset edit/drag → emit repaint. This is the private `activate` function in `workbook.rs`. When adding a new workbook-level operation (e.g. duplicate, open from file), put the shared transition there rather than duplicating it in the new arm.

### Adding a new workbook-level operation

1. Add a variant to `WorkbookAction`.
2. Add a match arm in `execute_workbook`. Call `activate` if the new operation loads a model.
3. Call `app.bump_registry()` if the operation changes which workbooks exist.

```rust
// Example: duplicate the active workbook
WorkbookAction::Duplicate => {
    let new_uuid = WorkbookId::new();
    let new_model = model.with_value(|m| m.clone_to_new("Copy"));
    storage::save(&new_uuid, &new_model);
    activate(new_uuid, new_model, model, state);
    app.bump_registry();
}
```

---

## Sheet-level actions (`SheetAction`)

`SheetAction` covers operations on sheets within the current workbook: select, add, delete, hide, unhide, rename, set color. These live in `src/input/sheet.rs` and are called directly from components — not routed through `classify_key`/`execute`.

### When to use `SheetAction` vs `StructAction`

Use `SheetAction` when the operation:
- Targets a sheet as a whole (not cells/rows/columns within it)
- Needs to persist to storage after the mutation
- Emits `Structure` or `Navigation` events related to sheet lifecycle

Use `StructAction` for cell, row, and column mutations within the active sheet.

### Calling it from a component

```rust
use crate::input::sheet::{execute_sheet, SheetAction};

// Select a sheet
execute_sheet(&SheetAction::Select(sheet_idx), model, &state);

// Add a new sheet
execute_sheet(&SheetAction::Add, model, &state);

// Delete (caller confirms first)
if confirmed {
    execute_sheet(&SheetAction::Delete(sheet_idx), model, &state);
}

// Rename
execute_sheet(
    &SheetAction::Rename { sheet: sheet_idx, name: new_name },
    model,
    &state,
);
```

`execute_sheet` takes the same `model` and `&state` that components already hold from `expect_context`. Unlike `execute_workbook`, it does not need `AppState` — sheet ops don't touch the workbook registry.

### Confirmation dialogs stay in the component

Same pattern as `WorkbookAction::Delete` — the `window.confirm()` call stays in the UI component. `SheetAction::Delete` assumes the user already confirmed.

### Adding a new sheet-level operation

1. Add a variant to `SheetAction` in `sheet.rs`.
2. Add a match arm in `execute_sheet`. Auto-save is handled centrally by the EventBus-driven auto-save in `app.rs` — no manual save call needed.
3. Emit the appropriate `SpreadsheetEvent` at the end of the arm.
