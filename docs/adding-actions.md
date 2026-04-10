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
```

### File layout

```
src/input/
├── action.rs        SpreadsheetAction, classify_key(), execute(), convenience constructors
├── error.rs         FormatError, StructError, NavError, EditError  (thiserror-derived)
├── nav.rs           NavAction      (arrows, page, home/end, sheet switch, select all)
├── edit.rs          EditAction     (start, commit, cancel)
├── format.rs        FormatAction   (bold, italic, underline, strikethrough, font size/family)
├── structure.rs     StructAction   (delete, clear, undo/redo, insert/delete rows/columns)
├── formula_input.rs (point-mode reference handling, separate from the action pipeline)
└── xlsx_io.rs       File import/export

src/model/
└── frontend_model.rs  mutate(), try_mutate(), EvaluationMode  (pause/resume wrappers)
```

### Two things bypass `execute()`

- **Clipboard** (`Copy`/`Cut`/`Paste`) needs the `AppClipboard` store and async OS clipboard APIs. Handled inline in `workbook.rs`.
- **Point-mode arrows** need the textarea cursor position from the DOM. Runs as a pre-check in `workbook.rs` before `classify_key` is called.

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

Errors propagate up to `execute()` in `action.rs`, which maps them all to
`String` and logs a single console warning. Callers of `execute()` never see
individual error types.

### 4. (Optional) Add a keyboard shortcut

If the action has a shortcut, add a branch to `classify_key()` in `action.rs`.
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

- To change the key binding: edit only `classify_key()` in `action.rs`.
- To change what it does: edit only the `execute_*()` function in the category file.
- To rename a variant: update the sub-enum and its execute arm. If it has a
  keyboard shortcut, update `classify_key()`. If it has a convenience
  constructor, update that too.

## Testing

Tests live in `action.rs` under `#[cfg(test)]`. Two kinds:

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
