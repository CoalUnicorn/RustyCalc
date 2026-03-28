# RustyCalc Rust Style Guide

This document captures the Rust design patterns and conventions used throughout RustyCalc. Following these patterns keeps the codebase consistent and leverages Rust's type system for correctness.

## Core Principles

### 1. Model the Domain in Types

**Use newtypes for domain values:**
```rust
// Good: Domain meaning encoded in type
pub struct CssColor(String);
impl CssColor {
    pub fn new(s: impl Into<String>) -> Self { /* validate */ }
}

// Avoid: Bare strings lose domain knowledge
fn set_color(color: String) { /* can't distinguish from other strings */ }
```

**Use enums for closed sets:**
```rust
// Good: Closed set of known font families
/// Font families the browser can reliably render.
/// Unknown font names from Excel files map to `SystemUi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SafeFontFamily {
    Arial,
    /// Renders as `"Calibri, system-ui"`. On Linux/Android, `system-ui` activates.
    CalibriLike,
    CourierNew,
    Georgia,
    TimesNewRoman,
    Verdana,
    /// Fallback for any unrecognised font name.
    SystemUi,
}

// Graceful handling of unknown inputs
impl From<Option<&str>> for SafeFontFamily {
    fn from(name: Option<&str>) -> Self {
        match name {
            Some("Arial") => Self::Arial,
            Some("Calibri") => Self::CalibriLike,
            Some("Courier New") => Self::CourierNew,
            Some("Georgia") => Self::Georgia,
            Some("Times New Roman") => Self::TimesNewRoman,
            Some("Verdana") => Self::Verdana,
            _ => Self::SystemUi,  // Unknown fonts safely fallback
        }
    }
}

// ❌ Avoid: String typing allows typos and invalid values
fn set_font(family: &str) { 
    /* "Ariell" compiles but is wrong, "Comic Sans" passes validation */ 
}
```

**Benefits of the enum approach:**
- **Compile-time safety**: Only valid fonts can be passed around
- **Exhaustive matching**: Adding a new font breaks the build until handled everywhere  
- **Graceful degradation**: Unknown fonts map to a safe fallback
- **Multiple contexts**: Same enum serves CSS, model storage, and UI display needs
- **Performance**: Zero-cost abstractions, optimizes to integers

### 2. Use Enums for State Machines

**Replace correlated booleans with state enums:**
```rust
// Good: Single enum makes illegal states unrepresentable
pub enum DragState {
    Idle,
    Selecting,
    Extending { to_row: i32, to_col: i32 },
    ResizingCol { col: i32, x: f64 },
}

// Avoid: Multiple booleans allow impossible states
struct BadState {
    selecting: bool,
    extending: bool, 
    resizing: bool,  // all three could be true!
}
```

### 3. Parse Don't Validate

**Parse once at boundaries, use domain types internally:**
```rust
// Good: Validation at construction ensures invariants
impl CssColor {
    pub fn new(s: impl Into<String>) -> Self {
        let s = s.into();
        if s.is_empty() { 
            Self("#000000".to_owned()) 
        } else { 
            Self(s) 
        }
    }
}

// Avoid: Validation functions that discard proof
fn validate_color(s: &str) -> Result<(), Error> { /* caller forgets result */ }
```

### 4. Exhaustive Matching

**List every variant for enums you control:**
```rust
// Good: Adding variants breaks the build
match edit_mode {
    EditMode::Accept => { /* handle */ }
    EditMode::Edit => { /* handle */ }
}

// Avoid: Wildcards hide missing cases
match edit_mode {
    EditMode::Accept => { /* handle */ }
    _ => {} // new variants slip through silently
}
```

### 5. Error Handling

**Use structured errors in libraries, log-and-continue for UI:**
```rust
// Good: Structured errors for recoverable cases
#[derive(Debug)]
pub enum WorkbookError {
    InvalidSheet(u32),
    ParseFailed(String),
}

// Good: Log-and-continue for UI interactions
pub fn warn_if_err<E: std::fmt::Display>(result: Result<(), E>, ctx: &str) {
    if let Err(e) = result {
        web_sys::console::warn_1(&format!("[ironcalc] {ctx}: {e}").into());
    }
}
```

### 6. Ownership and Borrowing

**Borrow by default, own when intentional:**
```rust
// Good: Borrow when you only read
fn process_text(text: &str) -> String { /* ... */ }

// Good: Own when storing or transforming
fn store_name(self, name: String) -> Self { 
    self.name = name; 
    self 
}
```

### 7. Collections and Structures

**Single struct per entity, not parallel collections:**
```rust
// Good: Related data grouped together
type Registry = HashMap<String, WorkbookMeta>;

struct WorkbookMeta {
    name: String,
    // other metadata fields...
}

// Avoid: Parallel collections can drift out of sync  
struct BadRegistry {
    names: HashMap<String, String>,
    timestamps: HashMap<String, DateTime>, // same keys, different maps
}
```

### 8. Module Organization

**Use modules for namespacing, not impl blocks:**
```rust
// Good: Module groups related functionality
pub mod geometry {
    pub fn cell_to_pixels(row: i32, col: i32) -> (f64, f64) { /* ... */ }
    pub fn pixels_to_cell(x: f64, y: f64) -> (i32, i32) { /* ... */ }
}

// Avoid: Unit structs as namespaces
struct Geometry;
impl Geometry {
    fn cell_to_pixels(row: i32, col: i32) -> (f64, f64) { /* ... */ }
}
```

### 9. Data-Driven Enum Implementations

**For enums with multiple output formats, use a data-driven approach:**
```rust
// Good: Single match block with structured data
#[derive(Debug, Clone, Copy)]
struct FontNames {
    css: &'static str,
    model: &'static str,
    label: &'static str,
}

impl SafeFontFamily {
    fn names(&self) -> FontNames {
        match self {
            Self::Arial => FontNames {
                css: "Arial",
                model: "Arial",
                label: "Arial",
            },
            Self::CalibriLike => FontNames {
                css: "Calibri, system-ui",
                model: "Calibri",
                label: "Calibri",
            },
            // ... other variants in one place
        }
    }

    pub fn css_name(&self) -> &'static str { self.names().css }
    pub fn model_name(&self) -> &'static str { self.names().model }
    pub fn label(&self) -> &'static str { self.names().label }
}

// ❌ Avoid: Multiple match blocks that can drift out of sync
impl SafeFontFamily {
    pub fn css_name(&self) -> &'static str {
        match self { /* 7 variants... */ }
    }
    pub fn model_name(&self) -> &'static str {
        match self { /* 7 variants again... */ }  
    }
    pub fn label(&self) -> &'static str {
        match self { /* 7 variants a third time... */ }
    }
}
```

**Benefits:**
- **Single source of truth**: Adding a variant requires editing one match block
- **Guaranteed consistency**: All output formats stay in sync
- **Zero runtime cost**: Compiles to the same optimized code
- **Clear data structure**: Shows exactly what each variant needs

## Component Patterns

### Signal Organization

**Group related signals in a state struct:**
```rust
#[derive(Clone, Copy)]
pub struct WorkbookState {
    pub editing_cell: RwSignal<Option<EditingCell>>,
    pub drag: RwSignal<DragState>,
    // ... other related signals
}
```

### Context Usage

**Use typed context instead of generic storage:**
```rust
// Good: Typed alias clarifies intent
pub type ModelStore = StoredValue<UserModel<'static>, LocalStorage>;

// Usage
let model = expect_context::<ModelStore>();
```

### Error Handling in Components

**Log errors but keep UI functional:**
```rust
let on_click = move |_| {
    model.update_value(|m| {
        warn_if_err(m.delete_sheet(sheet_id), "delete_sheet");
    });
    state.request_redraw();
};
```

## Examples from the Codebase

This style guide is derived from patterns already used throughout RustyCalc:

- **Domain types**: `CssColor`, `SafeFontFamily`, `ActiveCell`, `FrozenPanes`
- **State enums**: `EditMode`, `EditFocus`, `DragState`, `ArrowKey`  
- **Parse-don't-validate**: `CssColor::new()`, `SafeFontFamily::from()`
- **Data-driven enums**: `SafeFontFamily::names()` single match → multiple outputs
- **Structured collections**: `HashMap<String, WorkbookMeta>`
- **Error utilities**: `warn_if_err()`, storage error logging
- **Module organization**: `canvas::geometry`, `input::action`, `components::*`

These patterns make the code more maintainable, catch bugs at compile time, and express intent clearly through the type system.
