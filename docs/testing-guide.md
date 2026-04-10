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

#### 1. **Unit Tests** (Pure Functions)
Test individual functions without browser dependencies:

```rust
#[wasm_bindgen_test]
fn css_color_new_empty_substitutes_black() {
    let c = CssColor::new("");
    assert_eq!(c.as_str(), "#000000");
}

#[wasm_bindgen_test]
fn safe_font_family_unknown_falls_back() {
    assert_eq!(
        SafeFontFamily::from(Some("Comic Sans")),
        SafeFontFamily::SystemUi
    );
}
```

#### 2. **State Tests** (Leptos Signals)
Test state management with Leptos reactive system:

```rust
#[wasm_bindgen_test]
fn workbook_state_creation() {
    let state = WorkbookState::new();
    
    // Test initial values
    assert_eq!(state.drag.get(), DragState::Idle);
    assert!(state.editing_cell.get().is_none());
    
    // Test state updates
    state.editing_cell.set(Some(EditingCell {
        address: CellAddress { sheet: 1, row: 1, column: 1 },
        text: "test".to_owned(),
        mode: EditMode::Edit,
        focus: EditFocus::Cell,
    }));
    
    assert!(state.editing_cell.get().is_some());
}
```

#### 3. **Action Tests** (Action Dispatch System)
Test the action system with model mutations:

```rust
#[wasm_bindgen_test]
fn execute_navigate_down_advances_row() {
    // Use the test harness for setup
    let owner = Owner::new();
    owner.with(|| {
        let (model, state) = test_harness();
        
        // Execute action
        execute(&SpreadsheetAction::Nav(NavAction::Arrow(ArrowKey::Down)), model, &state);
        
        // Verify result
        let row = model.with_value(|m| m.get_selected_view().row);
        assert_eq!(row, 2);
    });
}

// Test helper function
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

#### 4. **Storage Tests** (LocalStorage Integration)
Test persistence layer:

```rust
#[wasm_bindgen_test] 
fn storage_save_load_roundtrip() {
    use crate::storage::*;
    
    // Create test model
    let model = ironcalc_base::UserModel::new_empty("test", "en", "UTC", "en").unwrap();
    let uuid = "test-uuid";
    
    // Save to storage
    save(uuid, &model);
    
    // Load from storage
    let loaded = load(uuid).expect("Should load successfully");
    
    // Verify data integrity
    assert_eq!(loaded.get_name(), "test");
}
```

#### 5. **Component Tests** (UI Components)
Test Leptos components:

```rust
use leptos::prelude::*;

#[wasm_bindgen_test]
fn toolbar_renders() {
    let owner = Owner::new();
    owner.with(|| {
        let (model, state) = test_harness();
        
        // Provide context for component
        provide_context(state);
        provide_context(model);
        
        // Mounting the component should not panic
        let _toolbar = view! { <Toolbar /> };
    });
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
- **Unit tests**: In same file as implementation (`#[cfg(test)]` module)
- **Integration tests**: In `src/input/action.rs` (action system tests)
- **Component tests**: Adjacent to component files
- **End-to-end tests**: Planned for `tests/` directory

## Best Practices

### 1. **Use Test Harness for Setup**
Avoid repeating model/state creation:

```rust
// Better: Use shared test harness
let (model, state) = test_harness();

// Verbose: Repeating setup in every test
let model = StoredValue::new_local(/* long setup */);
let state = WorkbookState::new();
```

### 2. **Test Error Conditions**
```rust
#[wasm_bindgen_test]
fn handles_invalid_input() {
    let result = SafeFontFamily::from(Some(""));
    assert_eq!(result, SafeFontFamily::SystemUi);
}
```

### 3. **Test Performance Assumptions**
Verify that `mutate()` optimizations work:

```rust
#[wasm_bindgen_test]
fn mutate_uses_single_evaluation() {
    let (model, state) = test_harness();
    
    // This should not cause double evaluation
    mutate(model, EvaluationMode::Immediate, |m| {
        warn_if_err(m.set_cell_value(1, 1, 1, "test"), "test");
    });
    
    // Test passes if no panic from double evaluation
}
```

### 4. **Clear Test Names**
```rust
// Better: Descriptive test names
#[wasm_bindgen_test]
fn css_color_empty_input_defaults_to_black() { /* ... */ }

// Problematic: Unclear names
#[wasm_bindgen_test]
fn test1() { /* ... */ }
```

### 5. **Isolate Tests**
Each test should be independent:

```rust
#[wasm_bindgen_test]
fn independent_test() {
    // Create fresh state for each test
    let (model, state) = test_harness();
    
    // Don't rely on global state or previous tests
    // ...
}
```

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

### Testing Enums
```rust
#[wasm_bindgen_test]
fn safe_font_family_css_names_non_empty() {
    // Test each variant explicitly - exhaustive match keeps this in sync
    let families = [
        SafeFontFamily::Arial,
        SafeFontFamily::CalibriLike,
        SafeFontFamily::CourierNew,
        SafeFontFamily::Georgia,
        SafeFontFamily::TimesNewRoman,
        SafeFontFamily::Verdana,
        SafeFontFamily::SystemUi,
    ];
    for family in families {
        assert!(!family.css_name().is_empty());
        assert!(!family.model_name().is_empty());
        assert!(!family.label().is_empty());
    }
}
```

### Testing Error Handling
```rust
#[wasm_bindgen_test]
fn warn_if_err_logs_errors() {
    // This should not panic, just log
    warn_if_err(Err("test error"), "test context");
    
    // Test passes if no panic occurred
}
```

### Testing State Transitions
```rust
#[wasm_bindgen_test]
fn drag_state_transitions() {
    let state = WorkbookState::new();
    
    // Initial state
    assert_eq!(state.drag.get(), DragState::Idle);
    
    // Transition to selecting
    state.drag.set(DragState::Selecting);
    assert_eq!(state.drag.get(), DragState::Selecting);
    
    // Transition to extending
    state.drag.set(DragState::Extending { to_row: 5, to_col: 3 });
    match state.drag.get() {
        DragState::Extending { to_row, to_col } => {
            assert_eq!(to_row, 5);
            assert_eq!(to_col, 3);
        }
        _ => panic!("Expected Extending state"),
    }
}
```

## Performance Testing

### Avoiding Double Evaluation
```rust
#[wasm_bindgen_test]
fn mutate_prevents_double_evaluation() {
    let (model, state) = test_harness();
    
    mutate(model, EvaluationMode::Immediate, |m| {
        warn_if_err(m.set_cell_value(1, 1, 1, "=A1"), "set_formula");
        warn_if_err(m.set_cell_value(1, 2, 1, "test"), "set_value");
    });
    
    // Test should complete without performance issues
}
```

## Running Tests in CI/CD

For continuous integration:

```bash
# In GitHub Actions or similar
- name: Run tests
  run: wasm-pack test --headless --firefox
```

## Future Improvements

Planned testing enhancements:
- Visual regression tests for canvas rendering
- Property-based testing with `proptest`
- Performance benchmarks
- End-to-end user interaction tests
- Accessibility testing

