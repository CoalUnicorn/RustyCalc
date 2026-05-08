# Testing Guide

RustyCalc uses `wasm-pack test` for browser-based testing. Tests run in a real browser with full DOM, LocalStorage, and Canvas 2D access.

## Test Setup and Running

### Prerequisites
```bash
# Install wasm-pack for WebAssembly testing
cargo install wasm-pack
```

### Running Tests
```bash
# Run all tests in headless browser
wasm-pack test --headless --firefox

# Run with Chrome (alternative)
wasm-pack test --headless --chrome

# Run in actual browser (for debugging)
wasm-pack test --firefox

# Run specific test
wasm-pack test --headless --firefox -- --test test_name
```

## Test Structure

### Basic Test Template
```rust
use wasm_bindgen_test::*;

// Configure for browser environment
wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn test_my_feature() {
    // Arrange
    let input = "test data";
    
    // Act
    let result = my_function(input);
    
    // Assert
    assert_eq!(result, expected_output);
}
```

### Test Categories

| Kind | What it tests | Setup |
|------|---------------|-------|
| **Unit** | Pure functions (`CssColor::new`, `SafeFontFamily::from`) | None — assert directly |
| **State** | `WorkbookState` signals, `DragState` transitions | `WorkbookState::new()` |
| **Action** | `execute()` dispatch with model mutations | `Owner::new()` + `test_harness()` |
| **Storage** | `localStorage` save/load roundtrip | Construct `UserModel`, call `save` / `load` |
| **Component** | Leptos `view!` mounts without panicking | `provide_context(state)` + `provide_context(model)` inside `Owner::new().with(...)` |

A representative Action test using the harness:

```rust
#[wasm_bindgen_test]
fn execute_navigate_down_advances_row() {
    let owner = Owner::new();
    owner.with(|| {
        let (model, state) = test_harness();
        execute(&SpreadsheetAction::Nav(NavAction::Arrow(ArrowKey::Down)), model, &state);
        assert_eq!(model.with_value(|m| m.get_selected_view().row), 2);
    });
}

#[cfg(test)]
fn test_harness() -> (ModelStore, WorkbookState) {
    (
        StoredValue::new_local(
            ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap(),
        ),
        crate::state::WorkbookState::new(),
    )
}
```

## Test Organization

### Module Structure
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;
    
    wasm_bindgen_test_configure!(run_in_browser);
    
    // Group related tests
    mod font_family_tests {
        use super::*;
        
        #[wasm_bindgen_test]
        fn css_names_correct() { /* ... */ }
        
        #[wasm_bindgen_test]
        fn model_names_correct() { /* ... */ }
    }
    
    mod action_tests {
        use super::*;
        
        #[wasm_bindgen_test]
        fn navigate_actions() { /* ... */ }
        
        #[wasm_bindgen_test] 
        fn format_actions() { /* ... */ }
    }
}
```

### Test File Locations
- **Unit tests**: in the same file as the implementation (`#[cfg(test)]` module)
- **Action dispatch / `classify_key` tests**: `src/input/keyboard.rs`
- **Pure-function tests** (no DOM): `src/input/formula_input.rs`
- **State tests**: `src/state.rs`
- **Component tests**: adjacent to component files

## Best Practices

- **Use `test_harness()`** for all action/state tests — keeps setup uniform.
- **Each test creates fresh state.** No shared mutables, no test ordering assumptions.
- **Name the assertion**, not the index. `css_color_empty_input_defaults_to_black` beats `test1`.
- **Clear `localStorage` at the top of storage tests** (`gloo_storage::LocalStorage::clear().ok();`) so prior runs can't leak state.

## Debugging Tests

### Browser Console
When tests fail, check browser console for error details:

```bash
# Run in actual browser to see console
wasm-pack test --firefox
```

### Console Logging
Add debug output in tests:

```rust
#[wasm_bindgen_test]
fn debug_test() {
    web_sys::console::log_1(&"Debug message".into());
    
    let value = some_function();
    web_sys::console::log_1(&format!("Value: {:?}", value).into());
    
    assert_eq!(value, expected);
}
```

### EventBus timing log

Debug event logging is wired into `emit_events()` but currently commented out. Uncomment the `leptos::logging::log!` call in `state.rs` to see per-event timestamps in the browser console. See [performance-evaluation.md](performance-evaluation.md) for details.

### Test Isolation
If tests interfere with each other:

```rust
#[wasm_bindgen_test]
fn isolated_test() {
    // Clear any global state
    gloo_storage::LocalStorage::clear().ok();
    
    // Run test logic
    // ...
}
```

## Common Patterns

**Exhaustive enum coverage:** iterate every variant in an array and assert per variant — adding a variant breaks the build until you append it.

```rust
#[wasm_bindgen_test]
fn safe_font_family_css_names_non_empty() {
    for family in [SafeFontFamily::Arial, SafeFontFamily::CalibriLike, /* ... */ SafeFontFamily::SystemUi] {
        assert!(!family.css_name().is_empty());
        assert!(!family.model_name().is_empty());
    }
}
```

**State transitions:** `match` on the new variant rather than equality so the destructured fields are checked too:

```rust
state.drag.set(DragState::Extending { to_row: 5, to_col: 3 });
let DragState::Extending { to_row, to_col } = state.drag.get() else { panic!("expected Extending") };
assert_eq!((to_row, to_col), (5, 3));
```

## Running Tests in CI/CD

The current `.github/workflows/rustycalc.yml` runs `cargo fmt`, `clippy`, and `check` on `wasm32-unknown-unknown`. Wasm-pack tests are not yet wired into CI. To add them:

```yaml
- name: Run tests
  run: wasm-pack test --headless --firefox
```

